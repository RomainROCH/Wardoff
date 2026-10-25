//! Runs the production IPC server on process-unique pipes with an inert UI consumer.
//! No blocker, autostart, singleton, logger, or production endpoint is started.
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::Threading::GetCurrentThreadId;
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
    use std::sync::Mutex;
    static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    pub(crate) enum EventSource {
        Ipc,
    }

    pub(crate) fn log_event(_: &str, _: EventSource, message: impl Into<String>, _: bool) {
        EVENTS
            .lock()
            .expect("inert log capture")
            .push(message.into());
    }
    pub(crate) fn contains(value: &str) -> bool {
        EVENTS
            .lock()
            .expect("inert log capture")
            .iter()
            .any(|message| message.contains(value))
    }
}

mod session_scope {
    pub(crate) fn current_control_pipe_path() -> Result<String, String> {
        Ok(format!(
            r"\\.\pipe\WardoffIpcTest-Control-{}",
            std::process::id()
        ))
    }

    pub(crate) fn current_status_pipe_path() -> Result<String, String> {
        Ok(format!(
            r"\\.\pipe\WardoffIpcTest-Status-{}",
            std::process::id()
        ))
    }
}

#[path = "../src/ipc.rs"]
mod ipc;

#[path = "../src/power_intent.rs"]
mod power_intent;

#[path = "../src/runtime_policy/sessions.rs"]
pub(crate) mod session_ledger;
mod runtime_policy {
    pub(crate) use crate::session_ledger as sessions;
}
#[path = "../src/session_owner.rs"]
mod session_owner;

struct TestRuntime {
    server: ipc::IpcServer,
    ui_thread: Option<JoinHandle<()>>,
    dispatched: Arc<AtomicUsize>,
    owners: Arc<Mutex<Vec<Option<session_ledger::OwnerKey>>>>,
}

impl TestRuntime {
    fn start() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<ipc::PendingRequest>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let dispatched = Arc::new(AtomicUsize::new(0));
        let ui_dispatched = Arc::clone(&dispatched);
        let owners = Arc::new(Mutex::new(Vec::new()));
        let ui_owners = Arc::clone(&owners);
        let ui_thread = thread::spawn(move || {
            // PostThreadMessageW requires the target thread to own a message queue.
            let mut message = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            }
            ready_tx
                .send(unsafe { GetCurrentThreadId() })
                .expect("UI ready");
            for pending in request_rx {
                ui_owners
                    .lock()
                    .expect("owner receipts")
                    .push(pending.owner.as_ref().map(session_owner::SessionOwner::key));
                ui_dispatched.fetch_add(1, Ordering::SeqCst);
                let response = match pending.request {
                    ipc::IpcRequest::Status => ipc::IpcResponse::Status {
                        status: cli::StatusOutput {
                            state: "inert".into(),
                        },
                    },
                    _ => ipc::IpcResponse::Ok,
                };
                let _ = pending.response_tx.send(response);
            }
        });
        let server = ipc::IpcServer::start(request_tx, ready_rx.recv().expect("UI thread ID"))
            .expect("isolated IPC startup");
        Self {
            server,
            ui_thread: Some(ui_thread),
            dispatched,
            owners,
        }
    }

    fn stop(&mut self) {
        self.server.shutdown().expect("clean IPC shutdown");
        if let Some(ui_thread) = self.ui_thread.take() {
            ui_thread.join().expect("inert UI joined");
        }
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn open_pipe(status: bool) -> File {
    let path = if status {
        session_scope::current_status_pipe_path()
    } else {
        session_scope::current_control_pipe_path()
    }
    .expect("isolated path");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match OpenOptions::new().read(true).write(!status).open(&path) {
            Ok(pipe) => return pipe,
            Err(error) if Instant::now() >= deadline => panic!("connect {path}: {error}"),
            Err(_) => thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn read_line_with_deadline(pipe: &mut File) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut reply = Vec::new();
    loop {
        let mut buffer = [0; 512];
        let mut available = 0;
        // This test client is the sole reader, so observed bytes cannot be consumed elsewhere.
        unsafe {
            PeekNamedPipe(
                HANDLE(pipe.as_raw_handle()),
                None,
                0,
                None,
                Some(&mut available),
                None,
            )
        }
        .map_err(|error| error.to_string())?;
        let to_read = buffer.len().min(available as usize);
        if to_read != 0 {
            match pipe.read(&mut buffer[..to_read]) {
                Ok(0) => return Err("server disconnected before replying".into()),
                Ok(size) => {
                    reply.extend_from_slice(&buffer[..size]);
                    if reply.contains(&b'\n') {
                        return String::from_utf8(reply).map_err(|error| error.to_string());
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        if Instant::now() >= deadline {
            return Err("test client received no reply within 2 seconds".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn exchange(payload: &[u8]) -> Result<ipc::IpcResponse, String> {
    let mut pipe = open_pipe(false);
    // Match send_request's separate JSON and newline pipe messages.
    if let Some(json) = payload.strip_suffix(b"\n") {
        pipe.write_all(json).map_err(|error| error.to_string())?;
        pipe.write_all(b"\n").map_err(|error| error.to_string())?;
    } else {
        pipe.write_all(payload).map_err(|error| error.to_string())?;
    }
    serde_json::from_str(&read_line_with_deadline(&mut pipe)?).map_err(|error| error.to_string())
}

fn assert_normal_command() {
    assert!(matches!(
        exchange(b"{\"command\":\"set_mode\",\"mode\":\"allow\"}\n"),
        Ok(ipc::IpcResponse::Ok)
    ));
}

fn assert_error(payload: &[u8], expected: &str) {
    let reply = exchange(payload).expect("error response");
    assert!(matches!(reply, ipc::IpcResponse::Error { message } if message.contains(expected)));
    assert_normal_command();
}

#[test]
fn isolated_runtime_rejects_bad_clients_and_recovers() {
    let mut runtime = TestRuntime::start();
    let before = runtime.dispatched.load(Ordering::SeqCst);
    let status_path = session_scope::current_status_pipe_path().unwrap();
    let rejected = OpenOptions::new()
        .read(true)
        .write(true)
        .open(status_path)
        .expect_err("read-only status pipe must reject a control-capable client");
    assert_eq!(rejected.raw_os_error(), Some(5));
    assert_eq!(runtime.dispatched.load(Ordering::SeqCst), before);
    assert_normal_command();
    assert_eq!(runtime.owners.lock().unwrap().last(), Some(&None));
    // Caller fields cannot choose another process identity. The production
    // server captures this connection's actual process object instead.
    assert!(matches!(
        exchange(b"{\"command\":\"begin_session\",\"owner_pid\":1,\"creation_time\":1}\n"),
        Ok(ipc::IpcResponse::Ok)
    ));
    let owner = runtime
        .owners
        .lock()
        .unwrap()
        .last()
        .copied()
        .flatten()
        .expect("captured owner");
    assert_eq!(owner.pid, std::process::id());
    assert_ne!(owner.creation_time, 0);
    let sensitive_fixture = "fixture-argument-that-must-not-enter-logs";
    let bad = format!("{{\"command\":\"acquire_session\",\"key\":{{\"runtime_id\":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],\"generation\":0,\"id\":1}},\"spec\":{{\"duration_ms\":1000,\"reason\":\"Fixture\",\"intent\":\"{sensitive_fixture}\"}}}}\n");
    assert_error(bad.as_bytes(), "malformed");
    assert!(
        !logger::contains(sensitive_fixture),
        "malformed session content entered structured logs"
    );
    assert!(matches!(
        exchange(b"{\"command\":\"set_power_intent\",\"intent\":\"system_only\"}\n"),
        Ok(ipc::IpcResponse::Ok)
    ));
    assert!(matches!(
        exchange(b"{\"command\":\"set_mode\",\"mode\":\"block\"}\r\n"),
        Ok(ipc::IpcResponse::Ok)
    ));

    // A rejected client may keep its handle open and refuse to read the error.
    // The unknown variant diagnostic can be larger than the outbound pipe buffer.
    let mut unread = open_pipe(false);
    let command = "x".repeat(4070);
    let payload = format!("{{\"command\":\"{command}\"}}\n");
    unread
        .write_all(payload.as_bytes())
        .expect("large malformed write");
    assert_normal_command();
    drop(unread);
    for (payload, expected) in [
        (&b"not JSON\n"[..], "malformed"),
        (&b"\n"[..], "empty"),
        (&b"{\"command\":\"unknown\"}\n"[..], "malformed"),
        (
            &b"{\"command\":\"set_power_intent\",\"intent\":\"display_only\"}\n"[..],
            "malformed",
        ),
        (&b"\xff\n"[..], "malformed"),
    ] {
        assert_error(payload, expected);
    }

    // The cap counts wire bytes including LF, regardless of JSON whitespace or message size.
    let mut boundary = b"{\"command\":\"status\"}".to_vec();
    boundary.resize(4095, b' ');
    boundary.push(b'\n');
    assert!(matches!(
        exchange(&boundary),
        Ok(ipc::IpcResponse::Status { .. })
    ));
    assert_normal_command();
    boundary.insert(boundary.len() - 1, b' ');
    assert_error(&boundary, "exceeds 4096 bytes");
    assert_error(&vec![b' '; 4097], "exceeds 4096 bytes");

    // Small individual messages must not bypass the cumulative frame cap.
    let mut oversized = open_pipe(false);
    for _ in 0..9 {
        let _ = oversized.write_all(&[b' '; 512]);
    }
    let reply = read_line_with_deadline(&mut oversized).expect("fragmented oversized reply");
    drop(oversized);
    assert!(reply.contains("exceeds 4096 bytes"));
    assert_normal_command();

    // EOF never turns an unterminated control command into an accepted request.
    for payload in [
        &b""[..],
        &b"{\"command\":\"set_mode\",\"mode\":\"allow\"}"[..],
    ] {
        let before = runtime.dispatched.load(Ordering::SeqCst);
        let mut disconnected = open_pipe(false);
        disconnected
            .write_all(payload)
            .expect("write before disconnect");
        drop(disconnected);
        assert_normal_command();
        assert_eq!(runtime.dispatched.load(Ordering::SeqCst), before + 1);
    }

    // A syntactically valid request without its newline must expire while open.
    let started = Instant::now();
    let mut stalled = open_pipe(false);
    stalled
        .write_all(b"{\"command\":\"status\"}")
        .expect("partial write");
    let status_started = Instant::now();
    let mut status_pipe = open_pipe(true);
    let status = read_line_with_deadline(&mut status_pipe).expect("independent status response");
    let status_elapsed = status_started.elapsed();
    assert!(
        status_elapsed < Duration::from_millis(500),
        "status waited {status_elapsed:?}"
    );
    eprintln!("status during incomplete request: {status_elapsed:?}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&status).expect("status JSON")["state"],
        "inert"
    );
    drop(status_pipe);
    let reply = read_line_with_deadline(&mut stalled);
    drop(stalled); // Unblocks the original reader too, even if this assertion fails.
    assert!(
        matches!(reply.as_ref().ok().and_then(|line| serde_json::from_str::<ipc::IpcResponse>(line).ok()), Some(ipc::IpcResponse::Error { message }) if message.contains("timed out")),
        "{reply:?}"
    );
    let partial_elapsed = started.elapsed();
    assert!(partial_elapsed >= Duration::from_millis(800));
    assert!(partial_elapsed < Duration::from_millis(1500));
    eprintln!("unterminated request rejected: {partial_elapsed:?}");
    assert_normal_command();

    // A client sending nothing also expires; another queued client then succeeds.
    let mut silent = open_pipe(false);
    let normal = thread::spawn(assert_normal_command);
    let reply = read_line_with_deadline(&mut silent).expect("silent timeout reply");
    drop(silent);
    assert!(reply.contains("timed out"));
    normal
        .join()
        .expect("normal client recovered after timeout");

    // Trickle traffic must not reset the absolute deadline.
    let mut slow = open_pipe(false);
    let mut slow_writer = slow.try_clone().expect("slow writer handle");
    let started = Instant::now();
    let writer = thread::spawn(move || {
        for _ in 0..20 {
            if slow_writer.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(80));
        }
    });
    let reply = read_line_with_deadline(&mut slow);
    drop(slow);
    let elapsed = started.elapsed();
    writer.join().expect("slow writer joined");
    assert!(reply.expect("trickle timeout reply").contains("timed out"));
    assert!(elapsed < Duration::from_millis(1500));
    assert!(elapsed >= Duration::from_millis(800));
    eprintln!("trickle request rejected: {elapsed:?}");
    assert_normal_command();

    // Clean shutdown must still finish when the current control client is incomplete.
    let mut shutdown_stall = open_pipe(false);
    shutdown_stall
        .write_all(b"{")
        .expect("shutdown partial write");
    let (stopped_tx, stopped_rx) = mpsc::channel();
    let shutdown = thread::spawn(move || {
        runtime.stop();
        stopped_tx.send(()).expect("shutdown result");
    });
    let stopped = stopped_rx.recv_timeout(Duration::from_secs(2));
    drop(shutdown_stall);
    shutdown.join().expect("shutdown joined");
    stopped.expect("shutdown completed despite incomplete client");
    // Both owned endpoints are gone after orderly shutdown.
    for path in [
        session_scope::current_control_pipe_path(),
        session_scope::current_status_pipe_path(),
    ] {
        assert!(OpenOptions::new()
            .read(true)
            .open(path.expect("test path"))
            .is_err());
    }
}
