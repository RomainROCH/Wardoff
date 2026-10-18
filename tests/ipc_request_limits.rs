//! The production IPC servers on GUID/PID-owned pipes with an inert dispatcher.
//! No protection runtime, singleton, autostart, power request or user log starts.
#![cfg(windows)]
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::{Error as WindowsError, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PATH_NOT_FOUND, ERROR_PIPE_BUSY,
    HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT, WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile, FILE_FLAG_OVERLAPPED};
use windows::Win32::System::Com::CoCreateGuid;
use windows::Win32::System::Threading::{CreateEventW, GetCurrentThreadId, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_NOREMOVE};

mod blocker {
    pub(crate) enum BlockerMode {
        Block,
        Allow,
    }
}

mod cli {
    #[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
    pub(crate) struct StatusOutput {
        pub(crate) state: String,
    }

    impl StatusOutput {
        pub(crate) fn to_json(&self) -> Result<String, serde_json::Error> {
            serde_json::to_string(self)
        }
    }
}

mod logger {
    pub(crate) enum EventSource {
        Ipc,
    }

    pub(crate) fn log_event(name: &str, _: EventSource, message: impl Into<String>, _: bool) {
        super::events()
            .lock()
            .expect("owned event collector")
            .push((name.to_owned(), message.into()));
    }
}

mod session_scope {
    pub(crate) fn path(kind: &str, pid: u32) -> String {
        format!(
            r"\\.\pipe\WardoffRequestLimits-{}-{pid}-{kind}",
            std::env::var(super::NAMESPACE).expect("owned child namespace")
        )
    }

    pub(crate) fn current_control_pipe_path() -> Result<String, String> {
        Ok(path("Control", std::process::id()))
    }

    pub(crate) fn current_status_pipe_path() -> Result<String, String> {
        Ok(path("Status", std::process::id()))
    }
}

#[path = "../src/ipc.rs"]
mod ipc;

const CHILD: &str = "WARDOFF_REQUEST_LIMITS_CHILD";
const NAMESPACE: &str = "WARDOFF_REQUEST_LIMITS_NAMESPACE";
const RECEIPT: &str = "WARDOFF_REQUEST_LIMITS_RECEIPT";
const BASELINE: &str = "WARDOFF_IPC_BASELINE";
const CAP: usize = 4096;
const IO_LIMIT: Duration = Duration::from_secs(3);
const WATCHDOG: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(5);
const CANARY: &str = "isolated_request_watchdog_reaps_owned_server";
const NORMAL: &[u8] = b"{\"command\":\"set_mode\",\"mode\":\"allow\"}";

type Events = Mutex<Vec<(String, String)>>;

fn events() -> &'static Events {
    static EVENTS: OnceLock<Events> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn rejection_count() -> usize {
    events()
        .lock()
        .expect("owned events")
        .iter()
        .filter(|(name, _)| name == "ipc_request_rejected")
        .count()
}

fn wait_rejection(after: usize) -> String {
    let deadline = Instant::now() + IO_LIMIT;
    loop {
        let found = events()
            .lock()
            .expect("owned events")
            .iter()
            .filter(|(name, _)| name == "ipc_request_rejected")
            .nth(after)
            .map(|(_, message)| message.clone());
        if let Some(message) = found {
            return message;
        }
        assert!(Instant::now() < deadline, "missing server rejection event");
        thread::sleep(POLL);
    }
}

struct EventHandle(HANDLE);

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn windows_io(error: WindowsError) -> io::Error {
    match WIN32_ERROR::from_error(&error) {
        Some(code) => io::Error::from_raw_os_error(code.0 as i32),
        None => io::Error::other(error),
    }
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "owned raw client I/O deadline")
}

// These clients use real cancellable I/O; no synchronous PeekNamedPipe can stall
// the observer. Cancellation is drained while its buffer and OVERLAPPED live.
fn raw_io<F>(file: &File, deadline: Instant, start: F) -> io::Result<u32>
where
    F: FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
{
    if Instant::now() >= deadline {
        return Err(timed_out());
    }
    let event = EventHandle(
        unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(windows_io)?,
    );
    let mut operation = OVERLAPPED {
        hEvent: event.0,
        ..OVERLAPPED::default()
    };
    if Instant::now() >= deadline {
        return Err(timed_out());
    }
    let pending = match start(&mut operation) {
        Ok(()) => false,
        Err(error) if error.code() == ERROR_IO_PENDING.to_hresult() => true,
        Err(error) => return Err(windows_io(error)),
    };
    let handle = HANDLE(file.as_raw_handle());
    let mut transferred = 0;
    if pending {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let milliseconds = remaining.as_nanos().div_ceil(1_000_000);
        let waited =
            unsafe { WaitForSingleObject(event.0, milliseconds.min(u32::MAX as u128 - 1) as u32) };
        if waited != WAIT_OBJECT_0 {
            let error = if waited == WAIT_TIMEOUT {
                timed_out()
            } else {
                io::Error::last_os_error()
            };
            unsafe {
                let _ = CancelIoEx(handle, Some(&operation));
                let _ = GetOverlappedResult(handle, &operation, &mut transferred, true);
            }
            return Err(error);
        }
    }
    unsafe { GetOverlappedResult(handle, &operation, &mut transferred, true) }
        .map_err(windows_io)?;
    if Instant::now() >= deadline {
        return Err(timed_out());
    }
    Ok(transferred)
}

struct RawPipe(File);

impl RawPipe {
    fn connect(kind: &str, deadline: Instant) -> io::Result<Self> {
        let path = session_scope::path(kind, std::process::id());
        loop {
            if Instant::now() >= deadline {
                return Err(timed_out());
            }
            match OpenOptions::new()
                .read(true)
                .write(kind == "Control")
                .custom_flags(FILE_FLAG_OVERLAPPED.0)
                .open(&path)
            {
                Ok(file) => return Ok(Self(file)),
                Err(error)
                    if matches!(error.raw_os_error(), Some(code)
                        if code == ERROR_FILE_NOT_FOUND.0 as i32
                            || code == ERROR_PATH_NOT_FOUND.0 as i32
                            || code == ERROR_PIPE_BUSY.0 as i32) =>
                {
                    thread::sleep(POLL);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn write_all(&self, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
        while !bytes.is_empty() {
            let count = raw_io(&self.0, deadline, |operation| unsafe {
                WriteFile(
                    HANDLE(self.0.as_raw_handle()),
                    Some(bytes),
                    None,
                    Some(operation),
                )
            })? as usize;
            if count == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "owned raw write"));
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }

    fn read_line(&self, deadline: Instant) -> io::Result<Vec<u8>> {
        let mut reply = Vec::new();
        loop {
            let mut buffer = [0; 512];
            let count = raw_io(&self.0, deadline, |operation| unsafe {
                ReadFile(
                    HANDLE(self.0.as_raw_handle()),
                    Some(&mut buffer),
                    None,
                    Some(operation),
                )
            })? as usize;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "owned raw EOF",
                ));
            }
            let newline = buffer[..count].iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(count, |position| position + 1);
            if reply.len() + take > 16 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "owned reply cap",
                ));
            }
            reply.extend_from_slice(&buffer[..take]);
            if newline.is_some() {
                return Ok(reply);
            }
        }
    }
}

struct TestServer {
    server: Option<ipc::IpcServer>,
    dispatcher: Option<JoinHandle<()>>,
    done: mpsc::Receiver<()>,
    hold_status: Arc<AtomicBool>,
    parked: mpsc::Receiver<mpsc::Sender<ipc::IpcResponse>>,
    controls: Arc<AtomicUsize>,
    statuses: Arc<AtomicUsize>,
}

impl TestServer {
    fn start() -> Self {
        let (requests_tx, requests_rx) = mpsc::channel::<ipc::PendingRequest>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let (parked_tx, parked) = mpsc::channel();
        let hold_status = Arc::new(AtomicBool::new(false));
        let controls = Arc::new(AtomicUsize::new(0));
        let statuses = Arc::new(AtomicUsize::new(0));
        let ui_hold = Arc::clone(&hold_status);
        let ui_controls = Arc::clone(&controls);
        let ui_statuses = Arc::clone(&statuses);
        let dispatcher = thread::spawn(move || {
            let mut message = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            }
            ready_tx
                .send(unsafe { GetCurrentThreadId() })
                .expect("inert queue ready");
            for pending in requests_rx {
                let response = match pending.request {
                    ipc::IpcRequest::Status => {
                        ui_statuses.fetch_add(1, Ordering::SeqCst);
                        if ui_hold.swap(false, Ordering::SeqCst) {
                            parked_tx
                                .send(pending.response_tx)
                                .expect("owned status barrier");
                            continue;
                        }
                        inert_status()
                    }
                    ipc::IpcRequest::SetMode { .. } => {
                        ui_controls.fetch_add(1, Ordering::SeqCst);
                        ipc::IpcResponse::Ok
                    }
                    _ => ipc::IpcResponse::Error {
                        message: "inert dispatcher refuses this test command".into(),
                    },
                };
                let _ = pending.response_tx.send(response);
            }
            done_tx.send(()).expect("inert dispatcher stopped");
        });
        let thread_id = ready_rx.recv_timeout(IO_LIMIT).expect("inert thread ID");
        let server = ipc::IpcServer::start(requests_tx, thread_id).expect("owned server startup");
        Self {
            server: Some(server),
            dispatcher: Some(dispatcher),
            done,
            hold_status,
            parked,
            controls,
            statuses,
        }
    }

    fn control_count(&self) -> usize {
        self.controls.load(Ordering::SeqCst)
    }

    fn status_count(&self) -> usize {
        self.statuses.load(Ordering::SeqCst)
    }

    fn stop_cleanly(mut self) {
        // Keep the existing dev status-shutdown replacement race outside this
        // fixture: park its real worker in response_rx.recv before setting the flag.
        self.hold_status.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + IO_LIMIT;
        let status = RawPipe::connect("Status", deadline).expect("status shutdown barrier client");
        let response = self
            .parked
            .recv_timeout(IO_LIMIT)
            .expect("status worker parked");
        self.server.as_ref().expect("owned server").begin_shutdown();
        response
            .send(inert_status())
            .expect("release parked status worker");
        let bytes = status
            .read_line(deadline)
            .expect("finish shutdown status response");
        assert_eq!(
            serde_json::from_slice::<cli::StatusOutput>(&bytes)
                .expect("status JSON")
                .state,
            "inert"
        );
        drop(status);
        self.server
            .as_mut()
            .expect("owned server")
            .shutdown()
            .expect("clean server shutdown");
        let _ = self.server.take();
        self.done
            .recv_timeout(IO_LIMIT)
            .expect("dispatcher exits after server senders drop");
        self.dispatcher
            .take()
            .expect("owned dispatcher")
            .join()
            .expect("dispatcher joined");
        assert_endpoints_removed(std::process::id());
        eprintln!("[request-limits] production workers and dispatcher joined; endpoints removed");
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Never hide a failed test in an unbounded Drop join. Only the success
        // path above claims clean shutdown; the parent owns/reaps this process.
        if self.server.is_some() {
            eprintln!("[request-limits] fixture did not finish clean shutdown; owned child cleanup required");
        }
    }
}

fn inert_status() -> ipc::IpcResponse {
    ipc::IpcResponse::Status {
        status: cli::StatusOutput {
            state: "inert".into(),
        },
    }
}

fn response(pipe: &RawPipe, deadline: Instant) -> Result<ipc::IpcResponse, String> {
    let bytes = pipe
        .read_line(deadline)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn exchange(parts: &[&[u8]], deadline: Instant) -> Result<ipc::IpcResponse, String> {
    let pipe = RawPipe::connect("Control", deadline).map_err(|error| error.to_string())?;
    for bytes in parts {
        pipe.write_all(bytes, deadline)
            .map_err(|error| error.to_string())?;
    }
    response(&pipe, deadline)
}

fn normal() {
    let result = exchange(&[NORMAL, b"\n"], Instant::now() + IO_LIMIT);
    assert!(
        matches!(result, Ok(ipc::IpcResponse::Ok)),
        "normal command: {result:?}"
    );
}

fn status() {
    let deadline = Instant::now() + IO_LIMIT;
    let pipe = RawPipe::connect("Status", deadline).expect("owned status connect");
    let bytes = pipe.read_line(deadline).expect("independent server status");
    let parsed = serde_json::from_slice::<cli::StatusOutput>(&bytes).expect("status JSON");
    assert_eq!(parsed.state, "inert");
}

fn assert_endpoints_removed(pid: u32) {
    for kind in ["Control", "Status"] {
        match OpenOptions::new()
            .read(true)
            .open(session_scope::path(kind, pid))
        {
            Err(error) => assert_eq!(
                error.raw_os_error(),
                Some(ERROR_FILE_NOT_FOUND.0 as i32),
                "owned {kind} cleanup"
            ),
            Ok(file) => {
                drop(file);
                panic!("owned {kind} endpoint survived");
            }
        }
    }
}

fn error_reply(result: Result<ipc::IpcResponse, String>, text: &str) {
    assert!(
        matches!(&result, Ok(ipc::IpcResponse::Error { message }) if message.contains(text)),
        "expected {text}: {result:?}"
    );
}

fn rejected(runtime: &TestServer, parts: &[&[u8]], text: &str) {
    let count = runtime.control_count();
    error_reply(exchange(parts, Instant::now() + IO_LIMIT), text);
    normal();
    assert_eq!(
        runtime.control_count(),
        count + 1,
        "rejection must not dispatch a command"
    );
}

fn boundary(ending: &[u8], total: usize) -> Vec<u8> {
    let mut bytes = b"{\"command\":\"status\"}".to_vec();
    bytes.resize(total - ending.len(), b' ');
    bytes.extend_from_slice(ending);
    bytes
}

fn valid_and_boundaries() {
    let runtime = TestServer::start();
    normal();
    for bytes in [boundary(b"\n", CAP), boundary(b"\r\n", CAP)] {
        let result = exchange(&[&bytes], Instant::now() + IO_LIMIT);
        assert!(
            matches!(result, Ok(ipc::IpcResponse::Status { .. })),
            "exact cap: {result:?}"
        );
        normal();
    }
    let utf8 = b"{\"command\":\"status\",\"note\":\"\xc3\xa9\"}\n";
    let split = utf8
        .iter()
        .position(|byte| *byte == 0xc3)
        .expect("split UTF8")
        + 1;
    assert!(matches!(
        exchange(&[&utf8[..split], &utf8[split..]], Instant::now() + IO_LIMIT),
        Ok(ipc::IpcResponse::Status { .. })
    ));
    normal();
    runtime.stop_cleanly();
}

fn malformed_and_oversized() {
    let runtime = TestServer::start();
    for (bytes, text) in [
        (&b"not JSON\n"[..], "malformed"),
        (&b"\n"[..], "empty"),
        (&b" \r\n"[..], "empty"),
        (&b"{\"command\":\"unknown\"}\n"[..], "malformed"),
        (&b"\xff\n"[..], "malformed"),
    ] {
        rejected(&runtime, &[bytes], text);
    }
    rejected(
        &runtime,
        &[b"{\"command\":\"status\",\"note\":\"\xc3", b"x\"}\n"],
        "malformed",
    );
    rejected(&runtime, &[&boundary(b"\n", CAP + 1)], "exceeds 4096 bytes");
    rejected(
        &runtime,
        &[&boundary(b"\r\n", CAP + 1)],
        "exceeds 4096 bytes",
    );
    // The cap is cumulative across messages, including a frame that cannot fit LF.
    let deadline = Instant::now() + IO_LIMIT;
    let pipe = RawPipe::connect("Control", deadline).expect("fragmented oversize client");
    for _ in 0..8 {
        pipe.write_all(&[b' '; 512], deadline)
            .expect("fragmented cap write");
    }
    error_reply(response(&pipe, deadline), "exceeds 4096 bytes");
    drop(pipe);
    normal();
    runtime.stop_cleanly();
}

fn eof_never_dispatches() {
    let runtime = TestServer::start();
    for bytes in [&b""[..], NORMAL, &b"{"[..]] {
        let rejected_before = rejection_count();
        let before = runtime.control_count();
        let deadline = Instant::now() + IO_LIMIT;
        let pipe = RawPipe::connect("Control", deadline).expect("EOF client");
        pipe.write_all(bytes, deadline).expect("EOF payload write");
        drop(pipe);
        let event = wait_rejection(rejected_before);
        assert!(
            event.contains(if bytes.is_empty() {
                "empty"
            } else {
                "without a terminating newline"
            }),
            "EOF rejection: {event}"
        );
        normal();
        assert_eq!(
            runtime.control_count(),
            before + 1,
            "EOF never dispatches unterminated control JSON"
        );
    }
    runtime.stop_cleanly();
}

fn timeouts_and_recovery() {
    let runtime = TestServer::start();
    let almost_full = vec![b' '; CAP - 1];
    for bytes in [&b""[..], &b"{"[..], NORMAL, &almost_full[..]] {
        let before = runtime.control_count();
        let rejected_before = rejection_count();
        let began = Instant::now();
        let deadline = began + IO_LIMIT;
        let stalled = RawPipe::connect("Control", deadline).expect("held incomplete client");
        stalled
            .write_all(bytes, deadline)
            .expect("incomplete write");
        status();
        assert_eq!(
            rejection_count(),
            rejected_before,
            "status must complete before control rejection"
        );
        error_reply(response(&stalled, deadline), "timed out");
        let elapsed = began.elapsed();
        assert!(
            elapsed >= Duration::from_millis(800) && elapsed < Duration::from_millis(2500),
            "request timeout: {elapsed:?}"
        );
        // Still hold the rejected client while the next command completes.
        normal();
        assert_eq!(runtime.control_count(), before + 1);
        drop(stalled);
        eprintln!("[request-limits] incomplete bytes={} timeout={elapsed:?}, status independent, recovery before close", bytes.len());
    }
    runtime.stop_cleanly();
}

fn trickle_has_one_deadline() {
    let runtime = TestServer::start();
    let before = runtime.control_count();
    let deadline = Instant::now() + IO_LIMIT;
    let pipe = Arc::new(RawPipe::connect("Control", deadline).expect("trickle client"));
    let writer_pipe = Arc::clone(&pipe);
    let began = Instant::now();
    let writer = thread::spawn(move || {
        for byte in NORMAL.iter().copied().chain(std::iter::once(b'\n')) {
            if writer_pipe.write_all(&[byte], deadline).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(80));
        }
    });
    error_reply(response(&pipe, deadline), "timed out");
    let elapsed = began.elapsed();
    assert!(
        elapsed >= Duration::from_millis(800) && elapsed < Duration::from_millis(2500),
        "trickle renewed deadline: {elapsed:?}"
    );
    writer.join().expect("trickle writer joined");
    normal();
    assert_eq!(runtime.control_count(), before + 1);
    drop(pipe);
    runtime.stop_cleanly();
}

fn unread_rejection_does_not_stall() {
    let runtime = TestServer::start();
    let command = "x".repeat(4070);
    let bytes = format!("{{\"command\":\"{command}\"}}\n").into_bytes();
    assert!(bytes.len() <= CAP);
    let parse_error =
        serde_json::from_slice::<ipc::IpcRequest>(&bytes).expect_err("unknown command");
    let diagnostic = ipc::IpcResponse::Error {
        message: format!("Wardoff received malformed IPC JSON: {parse_error}"),
    };
    assert!(
        serde_json::to_vec(&diagnostic)
            .expect("diagnostic JSON")
            .len()
            > CAP,
        "precondition: rejection exceeds outbound pipe buffer"
    );
    let before = runtime.control_count();
    let deadline = Instant::now() + IO_LIMIT;
    let unread = RawPipe::connect("Control", deadline).expect("nonreading client");
    unread
        .write_all(&bytes, deadline)
        .expect("large malformed payload");
    // Intentionally never read this response or close its client to release a
    // blocked write_response. Recovery must happen while the client stays open.
    normal();
    assert_eq!(runtime.control_count(), before + 1);
    status();
    drop(unread);
    runtime.stop_cleanly();
}

fn shutdown_with_held_partial() {
    let runtime = TestServer::start();
    let deadline = Instant::now() + IO_LIMIT;
    let partial = RawPipe::connect("Control", deadline).expect("shutdown partial client");
    partial
        .write_all(b"{", deadline)
        .expect("shutdown partial write");
    let began = Instant::now();
    runtime.stop_cleanly();
    assert!(
        began.elapsed() < Duration::from_millis(3500),
        "shutdown exceeded request bound plus connection overhead"
    );
    // The production workers and dispatcher have joined before this handle closes.
    drop(partial);
}

struct OwnedChild(Child);

impl OwnedChild {
    fn wait(&mut self, deadline: Instant) -> io::Result<Option<ExitStatus>> {
        loop {
            if let Some(status) = self.0.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn reap(&mut self) -> io::Result<()> {
        if self.0.try_wait()?.is_none() {
            if let Err(error) = self.0.kill() {
                if self.0.try_wait()?.is_none() {
                    return Err(error);
                }
            }
            self.wait(Instant::now() + Duration::from_secs(5))?
                .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "owned child cleanup"))?;
        }
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Err(error) = self.reap() {
            eprintln!("owned request server child cleanup failed: {error}");
        }
    }
}

struct Receipt(PathBuf);

impl Drop for Receipt {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            if error.kind() != io::ErrorKind::NotFound {
                eprintln!("owned request receipt cleanup failed: {error}");
            }
        }
    }
}

fn run_owned(test: &str, scenario: fn(), baseline: bool, canary: bool) {
    if baseline {
        assert_eq!(
            std::env::var(BASELINE).as_deref(),
            Ok("1"),
            "baseline probes require WARDOFF_IPC_BASELINE=1 and --ignored"
        );
    }
    if canary {
        assert_eq!(test, CANARY, "only the dedicated canary expects a kill");
    }
    if let Some(marker) = std::env::var_os(CHILD) {
        assert_eq!(marker.to_str(), Some("1"));
        let mut arguments = vec!["--exact", test, "--nocapture", "--test-threads=1"];
        if baseline {
            arguments.push("--ignored");
        }
        assert_eq!(std::env::args().skip(1).collect::<Vec<_>>(), arguments);
        let receipt = PathBuf::from(std::env::var_os(RECEIPT).expect("reserved receipt"));
        assert_eq!(receipt.parent(), Some(std::env::temp_dir().as_path()));
        assert!(receipt
            .file_name()
            .expect("receipt name")
            .to_string_lossy()
            .starts_with("WardoffRequestLimits-"));
        assert_eq!(
            std::fs::metadata(&receipt).expect("reserved receipt").len(),
            0
        );
        scenario();
        std::fs::write(receipt, b"workers-joined-endpoints-removed\n")
            .expect("verified cleanup receipt");
        return;
    }
    assert!(std::env::var_os(RECEIPT).is_none());
    assert!(std::env::var_os(NAMESPACE).is_none());
    let namespace = format!(
        "{:?}",
        unsafe { CoCreateGuid() }.expect("owned namespace GUID")
    );
    let receipt =
        Receipt(std::env::temp_dir().join(format!("WardoffRequestLimits-{namespace}.receipt")));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt.0)
        .expect("reserve owned receipt");
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command.args(["--exact", test, "--nocapture", "--test-threads=1"]);
    if baseline {
        command.arg("--ignored");
    }
    let mut child = OwnedChild(
        command
            .env(CHILD, "1")
            .env(NAMESPACE, &namespace)
            .env(RECEIPT, &receipt.0)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn owned inert production server child"),
    );
    let pid = child.0.id();
    let watchdog = if canary {
        let ready = Instant::now() + IO_LIMIT;
        loop {
            if std::fs::read(&receipt.0).expect("canary receipt") == b"canary-ready\n" {
                break;
            }
            assert!(
                child.0.try_wait().expect("canary status").is_none(),
                "canary exited before server readiness"
            );
            assert!(Instant::now() < ready, "canary server never became ready");
            thread::sleep(POLL);
        }
        Duration::from_millis(250)
    } else {
        WATCHDOG
    };
    eprintln!("[request-limits] owned-child={pid} watchdog={watchdog:?}");
    let status = child
        .wait(Instant::now() + watchdog)
        .expect("wait owned server child");
    child
        .reap()
        .expect("owned child killed/reaped if necessary");
    // Compute only this child's GUID/PID paths; never set aliases in the parent.
    for kind in ["Control", "Status"] {
        let path = format!(r"\\.\pipe\WardoffRequestLimits-{namespace}-{pid}-{kind}");
        match OpenOptions::new().read(true).open(path) {
            Err(error) => assert_eq!(
                error.raw_os_error(),
                Some(ERROR_FILE_NOT_FOUND.0 as i32),
                "owned child endpoint cleanup"
            ),
            Ok(file) => {
                drop(file);
                panic!("owned child {kind} endpoint survived");
            }
        }
    }
    eprintln!("[request-limits] child reaped; both owned endpoints absent");
    if canary {
        assert!(
            status.is_none(),
            "canary must actually exceed the short watchdog"
        );
        assert!(!child
            .0
            .try_wait()
            .expect("reaped canary")
            .expect("canary exit status")
            .success());
        assert_eq!(
            std::fs::read(&receipt.0).expect("canary receipt"),
            b"canary-ready\n"
        );
    } else {
        assert!(
            status
                .expect("owned server child exceeded watchdog")
                .success(),
            "owned server child failed; cleanup is not a passing test"
        );
        assert_eq!(
            std::fs::read(&receipt.0).expect("cleanup receipt"),
            b"workers-joined-endpoints-removed\n"
        );
    }
    let path = receipt.0.clone();
    drop(receipt);
    assert!(!path.exists(), "owned receipt survived cleanup");
    eprintln!("[request-limits] owned receipt removed; canary={canary}");
}

#[test]
fn isolated_request_valid_boundaries_and_utf8() {
    run_owned(
        "isolated_request_valid_boundaries_and_utf8",
        valid_and_boundaries,
        false,
        false,
    );
}

#[test]
fn isolated_request_malformed_empty_oversized_and_fragments() {
    run_owned(
        "isolated_request_malformed_empty_oversized_and_fragments",
        malformed_and_oversized,
        false,
        false,
    );
}

#[test]
fn isolated_request_eof_never_dispatches() {
    run_owned(
        "isolated_request_eof_never_dispatches",
        eof_never_dispatches,
        false,
        false,
    );
}

#[test]
fn isolated_request_timeouts_status_and_recovery() {
    run_owned(
        "isolated_request_timeouts_status_and_recovery",
        timeouts_and_recovery,
        false,
        false,
    );
}

#[test]
fn isolated_request_trickle_keeps_absolute_deadline() {
    run_owned(
        "isolated_request_trickle_keeps_absolute_deadline",
        trickle_has_one_deadline,
        false,
        false,
    );
}

#[test]
fn isolated_request_unread_error_does_not_stall_control() {
    run_owned(
        "isolated_request_unread_error_does_not_stall_control",
        unread_rejection_does_not_stall,
        false,
        false,
    );
}

#[test]
fn isolated_request_shutdown_joins_before_partial_client_closes() {
    run_owned(
        "isolated_request_shutdown_joins_before_partial_client_closes",
        shutdown_with_held_partial,
        false,
        false,
    );
}

#[test]
fn isolated_request_watchdog_reaps_owned_server() {
    run_owned(
        CANARY,
        || {
            let _runtime = TestServer::start();
            let receipt = PathBuf::from(std::env::var_os(RECEIPT).expect("canary receipt"));
            std::fs::write(receipt, b"canary-ready\n").expect("signal real server readiness");
            loop {
                thread::park();
            }
        },
        false,
        true,
    );
}

// These are intentionally ignored on every normal suite. On the original reader
// the valid probe passes and the other three fail after observations/clean stop.
// Run explicitly with WARDOFF_IPC_BASELINE=1, filter baseline_, and --ignored.
#[test]
#[ignore = "baseline-only observation: requires WARDOFF_IPC_BASELINE=1"]
fn baseline_request_valid_server() {
    run_owned(
        "baseline_request_valid_server",
        || {
            let runtime = TestServer::start();
            normal();
            status();
            runtime.stop_cleanly();
            eprintln!(
                "[request-baseline] valid control and independent status passed; clean shutdown"
            );
        },
        true,
        false,
    );
}

#[test]
#[ignore = "baseline-only expected failure: requires WARDOFF_IPC_BASELINE=1"]
fn baseline_request_silence_blocks_control() {
    run_owned(
        "baseline_request_silence_blocks_control",
        || {
            let runtime = TestServer::start();
            let stalled = RawPipe::connect("Control", Instant::now() + IO_LIMIT)
                .expect("baseline silent client");
            let began = Instant::now();
            status();
            let status_available = runtime.status_count() == 1;
            let normal_client = thread::spawn(|| {
                exchange(
                    &[NORMAL, b"\n"],
                    Instant::now() + Duration::from_millis(1800),
                )
            });
            let result = normal_client
                .join()
                .expect("bounded normal observer joined");
            let recovered_while_held = matches!(result, Ok(ipc::IpcResponse::Ok));
            while began.elapsed() < Duration::from_millis(2200) {
                thread::sleep(POLL);
            }
            eprintln!("[request-baseline] silence held={:?}, status_available={status_available}, control_recovered_before_close={recovered_while_held}, observer={result:?}", began.elapsed());
            drop(stalled);
            normal();
            runtime.stop_cleanly();
            eprintln!("[request-baseline] silence client closed; subsequent command passed; all workers joined");
            assert!(
                status_available,
                "status must remain independently available"
            );
            assert!(recovered_while_held, "EXPECTED BASELINE FAILURE: unbounded read monopolized control for >=2.2s until client close");
        },
        true,
        false,
    );
}

#[test]
#[ignore = "baseline-only expected failure: requires WARDOFF_IPC_BASELINE=1"]
fn baseline_request_oversized_is_rejected() {
    run_owned(
        "baseline_request_oversized_is_rejected",
        || {
            let runtime = TestServer::start();
            let result = exchange(&[&boundary(b"\n", CAP + 1)], Instant::now() + IO_LIMIT);
            eprintln!(
                "[request-baseline] offered_wire_bytes={} oversized_reply={result:?}",
                CAP + 1
            );
            normal();
            runtime.stop_cleanly();
            assert!(matches!(&result, Ok(ipc::IpcResponse::Error { message }) if message.contains("exceeds 4096 bytes")), "EXPECTED BASELINE FAILURE: 4097-byte request accepted or not classified as oversized: {result:?}");
        },
        true,
        false,
    );
}

#[test]
#[ignore = "baseline-only expected failure: requires WARDOFF_IPC_BASELINE=1"]
fn baseline_request_eof_valid_json_is_rejected() {
    run_owned(
        "baseline_request_eof_valid_json_is_rejected",
        || {
            let runtime = TestServer::start();
            let before = runtime.control_count();
            let deadline = Instant::now() + IO_LIMIT;
            let pipe = RawPipe::connect("Control", deadline).expect("baseline EOF client");
            pipe.write_all(NORMAL, deadline)
                .expect("baseline valid JSON without LF");
            drop(pipe);
            normal();
            let dispatched = runtime.control_count() - before;
            eprintln!("[request-baseline] EOF valid JSON without LF: dispatched={dispatched} including one subsequent normal command");
            runtime.stop_cleanly();
            assert_eq!(
                dispatched, 1,
                "EXPECTED BASELINE FAILURE: unterminated JSON dispatched at EOF"
            );
        },
        true,
        false,
    );
}
