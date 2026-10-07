//! Exercises production IPC clients/workers on process-unique, inert endpoints.
//! No protection runtime, singleton, autostart, logger, or production endpoint is started.
#![cfg(windows)]
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_OUTBOUND};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_NOREMOVE};

static MALFORMED_STATUS_REPLY: AtomicBool = AtomicBool::new(false);

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
            if super::MALFORMED_STATUS_REPLY.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok("not JSON".to_string());
            }
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
    use std::cell::RefCell;

    #[derive(Default)]
    struct ClientPaths {
        control: Option<String>,
        status: Option<String>,
    }

    thread_local! {
        static CLIENT_PATHS: RefCell<ClientPaths> = const {
            RefCell::new(ClientPaths { control: None, status: None })
        };
    }

    // Worker threads retain canonical names; aliases affect only this caller thread.
    pub(crate) struct ClientAliases(ClientPaths);

    impl ClientAliases {
        pub(crate) fn new(control: Option<String>, status: Option<String>) -> Self {
            Self(CLIENT_PATHS.with(|paths| paths.replace(ClientPaths { control, status })))
        }

        pub(crate) fn actual() -> Self {
            Self::new(None, None)
        }
    }

    impl Drop for ClientAliases {
        fn drop(&mut self) {
            CLIENT_PATHS.with(|paths| {
                paths.replace(std::mem::take(&mut self.0));
            });
        }
    }

    pub(crate) fn path(kind: &str) -> String {
        format!(
            r"\\.\pipe\WardoffStatusBudget-{kind}-{}",
            std::process::id()
        )
    }

    pub(crate) fn current_control_pipe_path() -> Result<String, String> {
        Ok(CLIENT_PATHS.with(|paths| {
            paths
                .borrow()
                .control
                .clone()
                .unwrap_or_else(|| path("Control"))
        }))
    }

    pub(crate) fn current_status_pipe_path() -> Result<String, String> {
        Ok(CLIENT_PATHS.with(|paths| {
            paths
                .borrow()
                .status
                .clone()
                .unwrap_or_else(|| path("Status"))
        }))
    }
}

#[path = "../src/ipc.rs"]
mod ipc;

struct TestRuntime {
    server: ipc::IpcServer,
    ui_thread: Option<JoinHandle<()>>,
    dispatched: Arc<AtomicUsize>,
}

impl TestRuntime {
    fn start() -> Self {
        let _actual = session_scope::ClientAliases::actual();
        let (request_tx, request_rx) = mpsc::channel::<ipc::PendingRequest>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let dispatched = Arc::new(AtomicUsize::new(0));
        let ui_dispatched = Arc::clone(&dispatched);
        let ui_thread = thread::spawn(move || {
            let mut message = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            }
            ready_tx
                .send(unsafe { GetCurrentThreadId() })
                .expect("UI ready");
            for pending in request_rx {
                ui_dispatched.fetch_add(1, Ordering::SeqCst);
                let response = match pending.request {
                    ipc::IpcRequest::Status => ipc::IpcResponse::Status {
                        status: cli::StatusOutput {
                            state: "allow".into(),
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
        }
    }

    fn stop(&mut self) {
        // Shutdown's own clients must never inherit a masked endpoint from a failed assertion.
        let _actual = session_scope::ClientAliases::actual();
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

struct MalformedReply;

impl MalformedReply {
    fn enable() -> Self {
        MALFORMED_STATUS_REPLY.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for MalformedReply {
    fn drop(&mut self) {
        MALFORMED_STATUS_REPLY.store(false, Ordering::SeqCst);
    }
}

// One connected instance makes a second open return ERROR_PIPE_BUSY without
// blocking a UI consumer or altering the production status server's lifecycle.
struct BusyStatusPipe {
    server: File,
    client: File,
}

impl BusyStatusPipe {
    fn new(path: &str) -> Self {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!handle.is_invalid(), "create isolated busy status pipe");
        let server = unsafe { File::from_raw_handle(handle.0 as RawHandle) };
        let client = OpenOptions::new()
            .read(true)
            .open(path)
            .expect("occupy status instance");
        match unsafe { ConnectNamedPipe(handle, None) } {
            Ok(()) => {}
            Err(_) if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED => {}
            Err(error) => panic!("connect occupied status instance: {error}"),
        }
        Self { server, client }
    }
}

// Mirrors the fallback decision only; the real CLI is checked separately.
fn query_status(deadline: Instant) -> Result<cli::StatusOutput, ipc::ClientError> {
    match ipc::read_status(Some(deadline)) {
        Err(ipc::ClientError::Unavailable) => {
            match ipc::send_status_request_with_connect_deadline(deadline)? {
                ipc::IpcResponse::Status { status } => Ok(status),
                _ => panic!("unexpected control status response"),
            }
        }
        result => result,
    }
}

fn assert_allow(result: Result<cli::StatusOutput, ipc::ClientError>) {
    match result {
        Ok(status) => assert_eq!(status.state, "allow"),
        Err(ipc::ClientError::Unavailable) => panic!("status was unavailable"),
        Err(ipc::ClientError::Transport(message)) => panic!("status transport: {message}"),
    }
}

fn wait_for_available(kind: &str) {
    let path = session_scope::path(kind);
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), 50) }.is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{kind} endpoint did not become available"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn isolated_status_connections_share_a_budget_and_preserve_fallback_rules() {
    // 1. Two absent endpoints share one short budget, not two retry windows.
    let started = Instant::now();
    let deadline = started + Duration::from_millis(140);
    assert!(matches!(
        ipc::read_status(Some(deadline)),
        Err(ipc::ClientError::Unavailable)
    ));
    assert!(
        Instant::now() >= deadline,
        "missing status should consume its retry budget"
    );
    let fallback_started = Instant::now();
    assert!(matches!(
        ipc::send_status_request_with_connect_deadline(deadline),
        Err(ipc::ClientError::Unavailable)
    ));
    let fallback_elapsed = fallback_started.elapsed();
    assert!(
        fallback_elapsed < Duration::from_millis(300),
        "expired absent control retried for {fallback_elapsed:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "absent endpoints renewed the budget"
    );
    eprintln!(
        "absent endpoints: {:?}; expired fallback: {fallback_elapsed:?}",
        started.elapsed()
    );

    // 2. A status server becoming ready within the budget is still reachable.
    let started = Instant::now();
    let launch = thread::spawn(|| {
        thread::sleep(Duration::from_millis(150));
        TestRuntime::start()
    });
    let status = ipc::read_status(Some(started + Duration::from_secs(2)));
    let mut runtime = launch.join().expect("delayed inert runtime started");
    assert_allow(status);
    assert!(started.elapsed() >= Duration::from_millis(100));

    // 3. An already expired deadline still permits one immediate open on either pipe.
    // Wait for the worker's replacement instance without consuming a connection.
    wait_for_available("Status");
    let expired = Instant::now() - Duration::from_millis(1);
    assert_allow(ipc::read_status(Some(expired)));
    wait_for_available("Control");
    match ipc::send_status_request_with_connect_deadline(expired) {
        Ok(ipc::IpcResponse::Status { status }) => assert_eq!(status.state, "allow"),
        _ => panic!("available control endpoint was skipped after deadline"),
    }

    // 4. A legacy control-only peer is usable after the missing status consumed the budget.
    let before = runtime.dispatched.load(Ordering::SeqCst);
    {
        let _alias =
            session_scope::ClientAliases::new(None, Some(session_scope::path("MissingStatus")));
        let started = Instant::now();
        assert_allow(query_status(started + Duration::from_millis(140)));
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_millis(700));
    }
    assert_eq!(runtime.dispatched.load(Ordering::SeqCst), before + 1);

    // 5. Malformed replies and busy status endpoints are transport failures, not fallback cues.
    let before = runtime.dispatched.load(Ordering::SeqCst);
    {
        let _malformed = MalformedReply::enable();
        assert!(
            matches!(query_status(Instant::now() + Duration::from_millis(140)),
            Err(ipc::ClientError::Transport(message)) if message.contains("malformed status JSON"))
        );
    }
    assert_eq!(
        runtime.dispatched.load(Ordering::SeqCst),
        before + 1,
        "malformed status triggered control fallback"
    );
    let busy_path = session_scope::path("BusyStatus");
    {
        let _busy = BusyStatusPipe::new(&busy_path);
        let _alias = session_scope::ClientAliases::new(None, Some(busy_path));
        assert!(
            matches!(query_status(Instant::now() + Duration::from_millis(140)),
            Err(ipc::ClientError::Transport(message)) if message.contains("busy"))
        );
    }
    assert_eq!(
        runtime.dispatched.load(Ordering::SeqCst),
        before + 1,
        "busy status triggered control fallback"
    );

    // 6. The ordinary command path and orderly cleanup retain their existing behavior.
    assert!(matches!(
        ipc::send_request(&ipc::IpcRequest::SetMode {
            mode: ipc::PipeMode::Allow
        }),
        Ok(ipc::IpcResponse::Ok)
    ));
    assert_allow(ipc::read_status(None));
    runtime.stop();
    for path in [
        session_scope::path("Control"),
        session_scope::path("Status"),
        session_scope::path("BusyStatus"),
    ] {
        assert!(
            OpenOptions::new().read(true).open(path).is_err(),
            "owned endpoint survived shutdown"
        );
    }
}
