//! Exercises production IPC clients/workers on process-unique, inert endpoints.
//! No protection runtime, singleton, autostart, logger, or production endpoint is started.
#![cfg(windows)]
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PIPE_CONNECTED};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_OUTBOUND};
use windows::Win32::System::Com::CoCreateGuid;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_NOREMOVE};

static MALFORMED_STATUS_REPLY: AtomicBool = AtomicBool::new(false);
const CHILD_MARKER: &str = "WARDOFF_STATUS_BUDGET_CHILD";
const COMPLETION_PATH: &str = "WARDOFF_STATUS_BUDGET_COMPLETION";
const TEST_NAME: &str = "isolated_status_connections_share_a_budget_and_preserve_fallback_rules";
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

macro_rules! trace {
    ($($arg:tt)*) => {
        // Format once: write_fmt can split a line across writes that interleave
        // with another child. Ignore diagnostic I/O errors even during unwind.
        let mut line = format!($($arg)*);
        line.push('\n');
        let _ = std::io::stderr().lock().write_all(line.as_bytes());
    };
}

// A successful libtest exit alone could mean the exact filter matched zero tests.
// Reserve a unique empty receipt before spawn; only the completed body fills it.
struct CompletionReceipt(PathBuf);

impl CompletionReceipt {
    fn new() -> Self {
        let guid = unsafe { CoCreateGuid() }.expect("create diagnostic receipt GUID");
        let path = std::env::temp_dir().join(format!("WardoffStatusBudget-{guid:?}.complete"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("reserve owned diagnostic receipt");
        Self(path)
    }
}

impl Drop for CompletionReceipt {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                trace!("[status-budget] owned receipt cleanup failed: {error}");
            }
        }
    }
}

fn completion_path() -> PathBuf {
    let receipt =
        PathBuf::from(std::env::var_os(COMPLETION_PATH).expect("owned completion receipt"));
    assert_eq!(receipt.parent(), Some(std::env::temp_dir().as_path()));
    assert!(receipt
        .file_name()
        .expect("receipt name")
        .to_string_lossy()
        .starts_with("WardoffStatusBudget-"));
    let metadata = std::fs::metadata(&receipt).expect("reserved completion receipt exists");
    assert!(metadata.is_file());
    assert_eq!(metadata.len(), 0, "completion receipt must start empty");
    receipt
}

fn write_completion(contents: &[u8]) {
    let receipt = completion_path();
    OpenOptions::new()
        .write(true)
        .open(receipt)
        .expect("open owned diagnostic receipt")
        .write_all(contents)
        .expect("record scenario progress");
}

fn spawn_child(completion: &CompletionReceipt, mode: &str) -> OwnedChild {
    OwnedChild(
        Command::new(std::env::current_exe().expect("current test executable"))
            .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD_MARKER, mode)
            .env(COMPLETION_PATH, &completion.0)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn isolated status-budget child"),
    )
}

// The inert runtime has synchronous production waits. Bound the entire scenario
// in a child process so a stalled wait or panic cleanup cannot strand its threads.
struct OwnedChild(Child);

impl OwnedChild {
    fn wait_until(&mut self, deadline: Instant) -> std::io::Result<Option<ExitStatus>> {
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

    fn kill_and_reap(&mut self) -> std::io::Result<()> {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return Ok(());
        }
        // This child starts only threads, never another process. Killing it closes
        // its isolated pipe handles; no production endpoint or other process is touched.
        if let Err(error) = self.0.kill() {
            // The child can exit between the poll and kill. A poll error must
            // still attempt termination rather than detach the owned process.
            if self.0.try_wait()?.is_some() {
                return Ok(());
            }
            return Err(error);
        }
        self.wait_until(Instant::now() + CLEANUP_TIMEOUT)?
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "child cleanup timed out")
            })?;
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Err(error) = self.kill_and_reap() {
            trace!("[status-budget] owned child cleanup failed: {error}");
        }
    }
}

struct Phase {
    name: &'static str,
    started: Instant,
}

impl Phase {
    fn begin(name: &'static str) -> Self {
        trace!(
            "[status-budget pid={}] phase={name} begin",
            std::process::id()
        );
        Self {
            name,
            started: Instant::now(),
        }
    }
}

impl Drop for Phase {
    fn drop(&mut self) {
        trace!(
            "[status-budget pid={}] phase={} {} elapsed={:?}",
            std::process::id(),
            self.name,
            if thread::panicking() { "unwind" } else { "end" },
            self.started.elapsed()
        );
    }
}

fn phase<T>(name: &'static str, operation: impl FnOnce() -> T) -> T {
    let _phase = Phase::begin(name);
    operation()
}

fn assert_owned_endpoints_removed(pid: u32) {
    for kind in ["Control", "Status", "BusyStatus"] {
        let path = session_scope::path_for_process(kind, pid);
        match OpenOptions::new().read(true).open(path) {
            Err(error) => assert_eq!(
                error.raw_os_error(),
                Some(ERROR_FILE_NOT_FOUND.0 as i32),
                "unexpected error checking {kind} endpoint cleanup"
            ),
            Ok(file) => {
                drop(file);
                panic!("owned {kind} endpoint survived child exit");
            }
        }
    }
}

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
        path_for_process(kind, std::process::id())
    }

    pub(crate) fn path_for_process(kind: &str, pid: u32) -> String {
        format!(r"\\.\pipe\WardoffStatusBudget-{kind}-{pid}")
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
            let _phase = Phase::begin("ui.consumer");
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
        let ui_thread_id = phase("runtime.ui-ready", || {
            ready_rx.recv().expect("UI thread ID")
        });
        let server = phase("runtime.ipc-start", || {
            ipc::IpcServer::start(request_tx, ui_thread_id).expect("isolated IPC startup")
        });
        Self {
            server,
            ui_thread: Some(ui_thread),
            dispatched,
        }
    }

    fn stop(&mut self) {
        // Shutdown's own clients must never inherit a masked endpoint from a failed assertion.
        let _actual = session_scope::ClientAliases::actual();
        phase("runtime.ipc-shutdown", || {
            self.server.shutdown().expect("clean IPC shutdown")
        });
        if let Some(ui_thread) = self.ui_thread.take() {
            phase("runtime.ui-join", || {
                ui_thread.join().expect("inert UI joined")
            });
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
        if unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), 50) }.as_bool() {
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
    if let Some(value) = std::env::var_os(CHILD_MARKER) {
        // An inherited marker on an ordinary Cargo invocation must fail before
        // starting a runtime instead of bypassing the parent's watchdog.
        assert!(
            std::env::args()
                .skip(1)
                .eq(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"].map(str::to_owned)),
            "child marker requires the exact owned invocation"
        );
        let _ = completion_path();
        if value == "cleanup-probe" {
            let _pipe = phase("diagnostic.busy-pipe-create", || {
                BusyStatusPipe::new(&session_scope::path("BusyStatus"))
            });
            write_completion(b"ready\n");
            let _phase = Phase::begin("diagnostic.intentional-wait");
            loop {
                thread::park();
            }
        }
        assert_eq!(value, "1", "invalid isolated child marker");
        phase("scenario", exercise_status_connection_budget);
        phase("scenario.complete", || write_completion(b"completed\n"));
        return;
    }

    let started = Instant::now();
    let completion = CompletionReceipt::new();
    let mut child = spawn_child(&completion, "1");
    let pid = child.0.id();
    trace!("[status-budget] child={pid} watchdog={SCENARIO_TIMEOUT:?}");
    let status = phase("parent.wait-child", || {
        child
            .wait_until(started + SCENARIO_TIMEOUT)
            .expect("poll owned child")
    });
    let Some(status) = status else {
        phase("parent.timeout-cleanup", || {
            child
                .kill_and_reap()
                .expect("terminate and reap owned child")
        });
        phase("parent.endpoint-cleanup", || {
            assert_owned_endpoints_removed(pid)
        });
        panic!("isolated status-budget scenario exceeded {SCENARIO_TIMEOUT:?}; inspect the child phase begin/end traces");
    };
    phase("parent.endpoint-cleanup", || {
        assert_owned_endpoints_removed(pid)
    });
    assert!(
        status.success(),
        "isolated status-budget scenario failed: {status}"
    );
    assert_eq!(
        std::fs::read(&completion.0).expect("read owned completion receipt"),
        b"completed\n",
        "child filter did not complete the scenario"
    );
    phase("parent.receipt-cleanup", || {
        std::fs::remove_file(&completion.0).expect("remove owned receipt")
    });
}

#[test]
fn diagnostic_watchdog_reaps_its_inert_child_and_closes_its_pipe() {
    let completion = CompletionReceipt::new();
    let mut child = spawn_child(&completion, "cleanup-probe");
    let pid = child.0.id();
    phase("diagnostic.wait-ready", || {
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        while std::fs::read(&completion.0).expect("read owned receipt") != b"ready\n" {
            assert!(
                child.0.try_wait().expect("poll probe").is_none(),
                "probe exited before ready"
            );
            assert!(Instant::now() < deadline, "probe startup timed out");
            thread::sleep(Duration::from_millis(10));
        }
    });
    assert!(phase("diagnostic.expected-timeout", || child
        .wait_until(Instant::now() + Duration::from_millis(20))
        .expect("poll probe"))
    .is_none());
    phase("diagnostic.kill-and-reap", || {
        child
            .kill_and_reap()
            .expect("terminate and reap inert probe")
    });
    phase("diagnostic.endpoint-cleanup", || {
        assert_owned_endpoints_removed(pid)
    });
    phase("diagnostic.receipt-cleanup", || {
        std::fs::remove_file(&completion.0).expect("remove owned probe receipt")
    });
}

fn exercise_status_connection_budget() {
    // 1. Two absent endpoints share one short budget, not two retry windows.
    let started = Instant::now();
    let deadline = started + Duration::from_millis(140);
    assert!(matches!(
        phase("absent.status-read", || ipc::read_status(Some(deadline))),
        Err(ipc::ClientError::Unavailable)
    ));
    assert!(
        Instant::now() >= deadline,
        "missing status should consume its retry budget"
    );
    let fallback_started = Instant::now();
    assert!(matches!(
        phase("absent.control-fallback", || {
            ipc::send_status_request_with_connect_deadline(deadline)
        }),
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
        let _phase = Phase::begin("runtime.delayed-start");
        thread::sleep(Duration::from_millis(150));
        TestRuntime::start()
    });
    let status = phase("delayed.status-read", || {
        ipc::read_status(Some(started + Duration::from_secs(2)))
    });
    let mut runtime = phase("delayed.join-startup", || {
        launch.join().expect("delayed inert runtime started")
    });
    assert_allow(status);
    assert!(started.elapsed() >= Duration::from_millis(100));

    // 3. An already expired deadline still permits one immediate open on either pipe.
    // Wait for the worker's replacement instance without consuming a connection.
    phase("expired.status-ready", || wait_for_available("Status"));
    let expired = Instant::now() - Duration::from_millis(1);
    assert_allow(phase("expired.status-read", || {
        ipc::read_status(Some(expired))
    }));
    phase("expired.control-ready", || wait_for_available("Control"));
    match phase("expired.control-read", || {
        ipc::send_status_request_with_connect_deadline(expired)
    }) {
        Ok(ipc::IpcResponse::Status { status }) => assert_eq!(status.state, "allow"),
        _ => panic!("available control endpoint was skipped after deadline"),
    }

    // 4. A legacy control-only peer is usable after the missing status consumed the budget.
    let before = runtime.dispatched.load(Ordering::SeqCst);
    {
        let _alias =
            session_scope::ClientAliases::new(None, Some(session_scope::path("MissingStatus")));
        let started = Instant::now();
        assert_allow(phase("legacy.control-fallback", || {
            query_status(started + Duration::from_millis(140))
        }));
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_millis(700));
    }
    assert_eq!(runtime.dispatched.load(Ordering::SeqCst), before + 1);

    // 5. Malformed replies and busy status endpoints are transport failures, not fallback cues.
    let before = runtime.dispatched.load(Ordering::SeqCst);
    {
        let _malformed = MalformedReply::enable();
        assert!(
            matches!(phase("malformed.status-read", || query_status(Instant::now() + Duration::from_millis(140))),
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
        let _busy = phase("busy.create-pipe", || BusyStatusPipe::new(&busy_path));
        let _alias = session_scope::ClientAliases::new(None, Some(busy_path));
        assert!(
            matches!(phase("busy.status-read", || query_status(Instant::now() + Duration::from_millis(140))),
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
        phase("normal.control-command", || ipc::send_request(
            &ipc::IpcRequest::SetMode {
                mode: ipc::PipeMode::Allow
            }
        )),
        Ok(ipc::IpcResponse::Ok)
    ));
    assert_allow(phase("normal.status-read", || ipc::read_status(None)));
    phase("normal.runtime-stop", || runtime.stop());
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
