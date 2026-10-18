//! Production status clients against process/GUID-unique, inert pipe fixtures.
//! No Wardoff runtime, singleton, blocker, autostart, or production endpoint starts.
#![cfg(windows)]
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    GetLastError, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_LISTENING, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, PIPE_ACCESS_OUTBOUND,
};
use windows::Win32::System::Com::CoCreateGuid;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PeekNamedPipe, PIPE_NOWAIT, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
};

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

    pub(crate) fn log_event(_: &str, _: EventSource, _: impl Into<String>, _: bool) {}
}

mod session_scope {
    pub(crate) fn path(kind: &str, pid: u32) -> String {
        format!(
            r"\\.\pipe\WardoffStatusReply-{}-{pid}-{kind}",
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

const CHILD: &str = "WARDOFF_STATUS_REPLY_CHILD";
const NAMESPACE: &str = "WARDOFF_STATUS_REPLY_NAMESPACE";
const RECEIPT: &str = "WARDOFF_STATUS_REPLY_RECEIPT";
const CAP: usize = 64 * 1024;
// Discovery on dev can consume about 1.9 seconds before the legacy reply budget starts.
const FIXTURE_LIMIT: Duration = Duration::from_secs(10);
const WATCHDOG: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(5);
const CANARY_TEST: &str = "isolated_status_reply_watchdog_reaps_owned_stall";

#[derive(Clone, Copy, Debug)]
enum Pathway {
    Status,
    Legacy,
}

struct Plan {
    chunks: Vec<(Duration, Vec<u8>)>,
    close: bool,
}

impl Plan {
    fn bytes(bytes: Vec<u8>, close: bool) -> Self {
        Self {
            chunks: vec![(Duration::ZERO, bytes)],
            close,
        }
    }

    fn silence() -> Self {
        Self {
            chunks: Vec::new(),
            close: false,
        }
    }
}

// Only the synthetic server uses NOWAIT. Production clients retain their rights/mode.
// Every fixture thread polls a finite deadline, is cancelled, and is joined.
struct Fixture {
    stop: Arc<AtomicBool>,
    connected: Arc<AtomicUsize>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

impl Fixture {
    fn start(kind: &str, request: bool, plan: Plan) -> Self {
        let path = session_scope::path(kind, std::process::id());
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                (if request {
                    PIPE_ACCESS_DUPLEX
                } else {
                    PIPE_ACCESS_OUTBOUND
                }) | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                (CAP * 3) as u32,
                4096,
                0,
                None,
            )
        };
        assert!(!handle.is_invalid(), "create owned {kind} fixture");
        let mut pipe = unsafe { File::from_raw_handle(handle.0 as RawHandle) };
        let stop = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicUsize::new(0));
        let worker_stop = Arc::clone(&stop);
        let worker_connected = Arc::clone(&connected);
        let worker = thread::spawn(move || {
            let until = Instant::now() + FIXTURE_LIMIT;
            loop {
                if worker_stop.load(Ordering::SeqCst) || Instant::now() >= until {
                    return Ok(());
                }
                let accepted = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) };
                let code = unsafe { GetLastError() };
                if accepted.is_ok() || code == ERROR_PIPE_CONNECTED {
                    worker_connected.fetch_add(1, Ordering::SeqCst);
                    break;
                }
                if code != ERROR_PIPE_LISTENING && code != ERROR_NO_DATA {
                    return Err(format!("fixture accept: {code:?}"));
                }
                thread::sleep(POLL);
            }
            if request && !read_fixture_request(&mut pipe, &worker_stop, until)? {
                return Ok(());
            }
            let began = Instant::now();
            for (offset, bytes) in plan.chunks {
                while Instant::now() < began + offset {
                    if worker_stop.load(Ordering::SeqCst) || Instant::now() >= until {
                        return Ok(());
                    }
                    thread::sleep(POLL);
                }
                if worker_stop.load(Ordering::SeqCst) || Instant::now() >= until {
                    return Ok(());
                }
                if let Err(error) = pipe.write_all(&bytes) {
                    if matches!(error.raw_os_error(), Some(code) if code == ERROR_BROKEN_PIPE.0 as i32 || code == ERROR_NO_DATA.0 as i32)
                    {
                        return Ok(());
                    }
                    return Err(format!("fixture write: {error}"));
                }
            }
            if !plan.close {
                while !worker_stop.load(Ordering::SeqCst) && Instant::now() < until {
                    thread::sleep(POLL);
                }
            }
            Ok(())
        });
        Self {
            stop,
            connected,
            worker: Some(worker),
        }
    }

    fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("owned fixture thread joined")
                .expect("owned fixture completed");
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.finish();
    }
}

fn read_fixture_request(
    pipe: &mut File,
    stop: &AtomicBool,
    until: Instant,
) -> Result<bool, String> {
    let mut request = Vec::new();
    while !stop.load(Ordering::SeqCst) && Instant::now() < until {
        let mut available = 0;
        if let Err(error) = unsafe {
            PeekNamedPipe(
                HANDLE(pipe.as_raw_handle()),
                None,
                0,
                None,
                Some(&mut available),
                None,
            )
        } {
            return if unsafe { GetLastError() } == ERROR_BROKEN_PIPE {
                Ok(false)
            } else {
                Err(format!("fixture request peek: {error}"))
            };
        }
        if available != 0 {
            // The real client sends this small request as JSON and LF messages.
            let mut buffer = [0; 512];
            let size = pipe.read(&mut buffer).map_err(|error| error.to_string())?;
            request.extend_from_slice(&buffer[..size]);
            assert!(request.len() < 512, "bounded inert fixture request");
            if request.contains(&b'\n') {
                return Ok(true);
            }
        }
        thread::sleep(POLL);
    }
    Ok(false)
}

fn query_status() -> Result<cli::StatusOutput, ipc::ClientError> {
    // Mirrors the CLI fallback decision; no CLI runtime or power operation runs.
    match ipc::read_status() {
        Err(ipc::ClientError::Unavailable) => legacy_status(),
        result => result,
    }
}

fn legacy_status() -> Result<cli::StatusOutput, ipc::ClientError> {
    match ipc::send_request(&ipc::IpcRequest::Status)? {
        ipc::IpcResponse::Status { status } => Ok(status),
        _ => panic!("unexpected inert legacy response"),
    }
}

fn payload(pathway: Pathway, state: &str) -> Vec<u8> {
    let status = cli::StatusOutput {
        state: state.into(),
    };
    match pathway {
        Pathway::Status => serde_json::to_vec(&status).expect("status JSON"),
        Pathway::Legacy => {
            serde_json::to_vec(&ipc::IpcResponse::Status { status }).expect("legacy status JSON")
        }
    }
}

fn assert_endpoints_removed(pid: u32) {
    for kind in ["Status", "Control"] {
        match OpenOptions::new()
            .read(true)
            .open(session_scope::path(kind, pid))
        {
            Err(error) => assert_eq!(
                error.raw_os_error(),
                Some(ERROR_FILE_NOT_FOUND.0 as i32),
                "owned {kind} endpoint cleanup"
            ),
            Ok(file) => {
                drop(file);
                panic!("owned {kind} endpoint survived fixture cleanup");
            }
        }
    }
}

fn exchange(
    pathway: Pathway,
    plan: Plan,
) -> (Result<cli::StatusOutput, ipc::ClientError>, Duration) {
    let (mut control, mut status) = match pathway {
        Pathway::Status => (
            Fixture::start("Control", true, Plan::silence()),
            Some(Fixture::start("Status", false, plan)),
        ),
        Pathway::Legacy => (Fixture::start("Control", true, plan), None),
    };
    let began = Instant::now();
    let result = match pathway {
        Pathway::Status => query_status(),
        Pathway::Legacy => legacy_status(),
    };
    let elapsed = began.elapsed();
    if let Some(fixture) = status.as_mut() {
        fixture.finish();
    }
    control.finish();
    let expected_control = usize::from(matches!(pathway, Pathway::Legacy));
    assert_eq!(
        control.connected.load(Ordering::SeqCst),
        expected_control,
        "status transport errors must never trigger legacy fallback"
    );
    assert_endpoints_removed(std::process::id());
    (result, elapsed)
}

fn assert_ok(result: Result<cli::StatusOutput, ipc::ClientError>, expected: &str) {
    match result {
        Ok(status) => assert_eq!(status.state, expected),
        Err(ipc::ClientError::Transport(message)) => panic!("unexpected transport: {message}"),
        Err(ipc::ClientError::Unavailable) => panic!("fixture unexpectedly unavailable"),
    }
}

fn recover(pathway: Pathway) {
    let mut bytes = payload(pathway, "recovered");
    bytes.push(b'\n');
    let (result, _) = exchange(pathway, Plan::bytes(bytes, false));
    assert_ok(result, "recovered");
}

fn assert_timeout(pathway: Pathway, name: &str, plan: Plan) {
    let (result, elapsed) = exchange(pathway, plan);
    eprintln!("[status-reply] {pathway:?} {name} elapsed={elapsed:?}");
    assert!(
        matches!(result, Err(ipc::ClientError::Transport(message)) if message.contains("timed out")),
        "{pathway:?} {name} must time out"
    );
    assert!(
        (Duration::from_millis(800)..Duration::from_millis(1800)).contains(&elapsed),
        "{pathway:?} {name} read deadline: {elapsed:?}"
    );
    recover(pathway);
}

fn exercise() {
    for pathway in [Pathway::Status, Pathway::Legacy] {
        for tail in [&b"\n"[..], &b"\r\n"[..], &b""[..]] {
            let state = "é".repeat(8);
            let mut exact = payload(pathway, &state);
            exact.resize(CAP - tail.len(), b' ');
            exact.extend_from_slice(tail);
            assert_eq!(exact.len(), CAP);
            let (result, _) = exchange(pathway, Plan::bytes(exact.clone(), tail.is_empty()));
            assert_ok(result, &state);
            eprintln!("[status-reply] {pathway:?} exact-wire-bytes={CAP} tail={tail:?}");
            exact.insert(exact.len() - tail.len(), b' ');
            let (result, _) = exchange(pathway, Plan::bytes(exact, tail.is_empty()));
            assert!(
                matches!(result, Err(ipc::ClientError::Transport(message)) if message.contains("exceeds"))
            );
            recover(pathway);
        }
        for malformed in [
            vec![],
            b"\n".to_vec(),
            b"not JSON\n".to_vec(),
            vec![0xff, b'\n'],
        ] {
            let (result, _) = exchange(pathway, Plan::bytes(malformed, true));
            assert!(matches!(result, Err(ipc::ClientError::Transport(_))));
            recover(pathway);
        }
        let mut bytes = payload(pathway, "fragment-é");
        bytes.extend_from_slice(b"\r\n");
        let split = bytes
            .iter()
            .position(|byte| *byte == 0xc3)
            .expect("UTF8 split");
        let fragments = Plan {
            chunks: vec![
                (Duration::ZERO, bytes[..split + 1].to_vec()),
                (Duration::from_millis(40), bytes[split + 1..].to_vec()),
            ],
            close: false,
        };
        assert_ok(exchange(pathway, fragments).0, "fragment-é");
        let mut first_line = payload(pathway, "first-line");
        first_line.push(b'\n');
        first_line.resize(CAP + 32, b' ');
        assert_ok(
            exchange(pathway, Plan::bytes(first_line, false)).0,
            "first-line",
        );
        let oversized = Plan {
            chunks: (0..17)
                .map(|_| (Duration::ZERO, vec![b' '; 4096]))
                .collect(),
            close: false,
        };
        assert!(
            matches!(exchange(pathway, oversized).0, Err(ipc::ClientError::Transport(message)) if message.contains("exceeds"))
        );
        recover(pathway);
        for (name, plan) in [
            ("silent", Plan::silence()),
            ("partial", Plan::bytes(b"{\"state\":".to_vec(), false)),
            (
                "trickle",
                Plan {
                    chunks: payload(pathway, "trickle")
                        .into_iter()
                        .chain(std::iter::once(b'\n'))
                        .enumerate()
                        .map(|(index, byte)| {
                            (Duration::from_millis(index as u64 * 120), vec![byte])
                        })
                        .collect(),
                    close: false,
                },
            ),
        ] {
            assert_timeout(pathway, name, plan);
        }
    }

    // Missing status uses dev's 20 connection attempts (19 pauses of 100 ms).
    // A legacy reply delayed by 600 ms still gets its fresh connected read budget.
    let mut bytes = payload(Pathway::Legacy, "legacy-after-discovery");
    bytes.push(b'\n');
    let mut legacy = Fixture::start(
        "Control",
        true,
        Plan {
            chunks: vec![(Duration::from_millis(600), bytes)],
            close: false,
        },
    );
    let began = Instant::now();
    assert_ok(query_status(), "legacy-after-discovery");
    let elapsed = began.elapsed();
    eprintln!("[status-reply] legacy-after-discovery elapsed={elapsed:?}");
    assert!(
        elapsed >= Duration::from_millis(2400),
        "legacy fallback must exercise discovery retries before the delayed reply: {elapsed:?}"
    );
    legacy.finish();
    assert_eq!(legacy.connected.load(Ordering::SeqCst), 1);
    assert_endpoints_removed(std::process::id());

    // The unchanged ordinary control path still returns Ok, and legacy errors remain values.
    for (reply, error) in [
        (b"{\"type\":\"ok\"}\n".to_vec(), false),
        (
            b"{\"type\":\"error\",\"message\":\"inert refusal\"}\n".to_vec(),
            true,
        ),
    ] {
        let mut control = Fixture::start("Control", true, Plan::bytes(reply, false));
        if error {
            assert!(
                matches!(ipc::send_request(&ipc::IpcRequest::Status), Ok(ipc::IpcResponse::Error { message }) if message == "inert refusal")
            );
        } else {
            assert!(matches!(
                ipc::send_request(&ipc::IpcRequest::SetMode {
                    mode: ipc::PipeMode::Allow
                }),
                Ok(ipc::IpcResponse::Ok)
            ));
        }
        control.finish();
        assert_endpoints_removed(std::process::id());
    }
}

struct OwnedChild(Child);

impl OwnedChild {
    fn wait(&mut self, deadline: Instant) -> std::io::Result<Option<ExitStatus>> {
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

    fn reap(&mut self) -> std::io::Result<()> {
        if self.0.try_wait()?.is_none() {
            if let Err(error) = self.0.kill() {
                if self.0.try_wait()?.is_none() {
                    return Err(error);
                }
            }
            self.wait(Instant::now() + Duration::from_secs(5))?
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::TimedOut, "owned child cleanup")
                })?;
        }
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Err(error) = self.reap() {
            eprintln!("owned status reply child cleanup failed: {error}");
        }
    }
}

struct Receipt(PathBuf);

impl Drop for Receipt {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("owned status reply receipt cleanup failed: {error}");
            }
        }
    }
}

fn run_owned(test: &str, scenario: fn()) {
    run_owned_with_watchdog(test, scenario, false);
}

fn run_owned_with_watchdog(test: &str, scenario: fn(), canary: bool) {
    if canary {
        assert_eq!(
            test, CANARY_TEST,
            "only the dedicated canary expects a kill"
        );
    }
    if let Some(marker) = std::env::var_os(CHILD) {
        assert_eq!(marker.to_str(), Some("1"));
        assert_eq!(
            std::env::args().skip(1).collect::<Vec<_>>(),
            ["--exact", test, "--nocapture", "--test-threads=1"]
        );
        let receipt = PathBuf::from(std::env::var_os(RECEIPT).expect("reserved receipt"));
        assert_eq!(receipt.parent(), Some(std::env::temp_dir().as_path()));
        assert!(receipt
            .file_name()
            .expect("receipt name")
            .to_string_lossy()
            .starts_with("WardoffStatusReply-"));
        assert_eq!(
            std::fs::metadata(&receipt).expect("reserved receipt").len(),
            0
        );
        scenario();
        std::fs::write(receipt, b"fixtures-joined-endpoints-removed\n")
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
        Receipt(std::env::temp_dir().join(format!("WardoffStatusReply-{namespace}.receipt")));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt.0)
        .expect("reserve owned receipt");
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env(NAMESPACE, &namespace)
            .env(RECEIPT, &receipt.0)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn owned inert status child"),
    );
    let pid = child.0.id();
    let watchdog = if canary {
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if std::fs::read(&receipt.0).expect("read owned canary receipt") == b"canary-ready\n" {
                break;
            }
            assert!(
                child
                    .0
                    .try_wait()
                    .expect("canary readiness status")
                    .is_none(),
                "canary exited before fixture readiness"
            );
            assert!(
                Instant::now() < ready_deadline,
                "canary fixture was never ready"
            );
            thread::sleep(POLL);
        }
        eprintln!("[status-reply] canary endpoint created; short watchdog armed");
        Duration::from_millis(250)
    } else {
        WATCHDOG
    };
    eprintln!("[status-reply] owned-child={pid} watchdog={watchdog:?}");
    let status = child
        .wait(Instant::now() + watchdog)
        .expect("wait owned child");
    child.reap().expect("owned child killed/reaped if needed");
    // Do not set process-wide aliases; parent computes only its owned child's endpoints.
    for kind in ["Status", "Control"] {
        let path = format!(r"\\.\pipe\WardoffStatusReply-{namespace}-{pid}-{kind}");
        match OpenOptions::new().read(true).open(path) {
            Err(error) => assert_eq!(error.raw_os_error(), Some(ERROR_FILE_NOT_FOUND.0 as i32)),
            Ok(file) => {
                drop(file);
                panic!("owned child {kind} endpoint survived");
            }
        }
    }
    if canary {
        assert!(
            status.is_none(),
            "canary must actually exceed the short watchdog"
        );
        assert!(!child
            .0
            .try_wait()
            .expect("reaped canary status")
            .expect("canary was reaped")
            .success());
        assert_eq!(
            std::fs::read(&receipt.0).expect("canary receipt"),
            b"canary-ready\n"
        );
        eprintln!("[status-reply] canary timed out, killed/reaped, owned endpoints removed");
    } else {
        assert!(
            status
                .expect("owned status child exceeded watchdog")
                .success(),
            "owned inert status child failed"
        );
        assert_eq!(
            std::fs::read(&receipt.0).expect("child cleanup receipt"),
            b"fixtures-joined-endpoints-removed\n"
        );
    }
    let receipt_path = receipt.0.clone();
    drop(receipt);
    assert!(!receipt_path.exists(), "owned receipt survived cleanup");
    eprintln!("[status-reply] owned child reaped, endpoints and receipt removed");
    if canary {
        eprintln!("[status-reply] canary owned receipt removal confirmed");
    }
}

#[test]
fn isolated_status_reply_watchdog_reaps_owned_stall() {
    run_owned_with_watchdog(
        CANARY_TEST,
        || {
            let _fixture = Fixture::start("Status", false, Plan::silence());
            let receipt = PathBuf::from(std::env::var_os(RECEIPT).expect("owned canary receipt"));
            std::fs::write(receipt, b"canary-ready\n").expect("signal created owned endpoint");
            loop {
                thread::park();
            }
        },
        true,
    );
}

#[test]
fn isolated_status_reply_limits_and_recovery() {
    run_owned("isolated_status_reply_limits_and_recovery", exercise);
}

// These independent probes also give the unmodified reader separate, bounded
// valid/silent/partial/oversized reproduction results instead of one fail-fast case.
#[test]
fn isolated_status_reply_valid_response() {
    run_owned("isolated_status_reply_valid_response", || {
        for pathway in [Pathway::Status, Pathway::Legacy] {
            recover(pathway);
        }
    });
}

#[test]
fn isolated_status_reply_silence_is_bounded() {
    run_owned("isolated_status_reply_silence_is_bounded", || {
        for pathway in [Pathway::Status, Pathway::Legacy] {
            assert_timeout(pathway, "silent", Plan::silence());
        }
    });
}

#[test]
fn isolated_status_reply_partial_is_bounded() {
    run_owned("isolated_status_reply_partial_is_bounded", || {
        for pathway in [Pathway::Status, Pathway::Legacy] {
            assert_timeout(
                pathway,
                "partial",
                Plan::bytes(b"{\"state\":".to_vec(), false),
            );
        }
    });
}

#[test]
fn isolated_status_reply_oversize_is_bounded() {
    run_owned("isolated_status_reply_oversize_is_bounded", || {
        for pathway in [Pathway::Status, Pathway::Legacy] {
            let mut oversized = payload(pathway, "oversized");
            oversized.resize(CAP, b' ');
            oversized.push(b'\n');
            let (result, _) = exchange(pathway, Plan::bytes(oversized, false));
            assert!(
                matches!(result, Err(ipc::ClientError::Transport(message)) if message.contains("exceeds")),
                "{pathway:?} oversized frame accepted"
            );
            recover(pathway);
        }
    });
}
