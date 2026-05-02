use crate::blocker::BlockerMode;
use crate::cli::StatusOutput;
use crate::logger::{self, EventSource};
use crate::session_scope::{current_control_pipe_path, current_status_pipe_path};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::mem::size_of;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Sender},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::{Error as WindowsError, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND,
    ERROR_INSUFFICIENT_BUFFER, ERROR_PATH_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE,
    HLOCAL, LPARAM, WPARAM,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{PIPE_ACCESS_DUPLEX, PIPE_ACCESS_OUTBOUND};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};

const IPC_SERVER_THREAD_NAME: &str = "wardoff-ipc-server";
const STATUS_SERVER_THREAD_NAME: &str = "wardoff-status-server";
const PIPE_BUFFER_SIZE: u32 = 4096;
const PIPE_CONNECT_ATTEMPTS: usize = 20;
const PIPE_CONNECT_DELAY: Duration = Duration::from_millis(100);
const CONTROL_PIPE_SECURITY_SDDL_PREFIX: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;";
const CONTROL_PIPE_SECURITY_SDDL_SUFFIX: &str = ")S:(ML;;NW;;;ME)";

/// Wakes the UI thread when the IPC server has queued a new request.
pub(crate) const IPC_WAKE_MESSAGE: u32 = WM_APP + 1;

/// Represents the block/allow mode transported across the named pipe.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PipeMode {
    /// Requests Block mode.
    Block,
    /// Requests Allow mode.
    Allow,
}

/// Represents the commands accepted by the primary Wardoff runtime.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum IpcRequest {
    /// Switches the runtime into the requested blocker mode.
    SetMode { mode: PipeMode },
    /// Creates, updates, or removes the current-user autostart task.
    SetAutostart { enabled: bool },
    /// Returns the current runtime status snapshot.
    Status,
    /// Stops the background named-pipe server during orderly shutdown.
    ShutdownServer,
}

/// Represents the serialized replies returned over the named pipe.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum IpcResponse {
    /// Acknowledges that a control command completed successfully.
    Ok,
    /// Returns the current runtime status snapshot.
    Status { status: StatusOutput },
    /// Returns an error string that the caller can surface to the user.
    Error { message: String },
}

/// Carries a parsed IPC request and the channel used to reply on the UI thread.
pub(crate) struct PendingRequest {
    /// The parsed request received over the named pipe.
    pub(crate) request: IpcRequest,
    /// The one-shot sender used to return the UI-thread response.
    pub(crate) response_tx: Sender<IpcResponse>,
}

/// Describes why an IPC client could not talk to the primary runtime.
pub(crate) enum ClientError {
    /// No primary runtime was reachable on the named pipe.
    Unavailable,
    /// A transport or protocol error occurred while using the named pipe.
    Transport(String),
}

/// Owns the named-pipe server thread used by the primary Wardoff runtime.
pub(crate) struct IpcServer {
    control_join_handle: Option<JoinHandle<()>>,
    status_join_handle: Option<JoinHandle<()>>,
    status_shutdown_requested: Arc<AtomicBool>,
}

impl IpcServer {
    /// Starts the background named-pipe server for the primary runtime.
    pub(crate) fn start(
        request_tx: Sender<PendingRequest>,
        ui_thread_id: u32,
    ) -> Result<Self, String> {
        let control_pipe_path = current_control_pipe_path()?;
        let status_pipe_path = current_status_pipe_path()?;
        let status_shutdown_requested = Arc::new(AtomicBool::new(false));
        let control_request_tx = request_tx.clone();
        let control_join_handle = thread::Builder::new()
            .name(IPC_SERVER_THREAD_NAME.to_string())
            .spawn(move || run_server_loop(control_request_tx, ui_thread_id))
            .map_err(|error| format!("Wardoff could not start its IPC server thread: {error}"))?;
        let status_join_handle = thread::Builder::new()
            .name(STATUS_SERVER_THREAD_NAME.to_string())
            .spawn({
                let shutdown_requested = Arc::clone(&status_shutdown_requested);
                move || run_status_server_loop(request_tx, ui_thread_id, shutdown_requested)
            })
            .map_err(|error| {
                format!("Wardoff could not start its status server thread: {error}")
            })?;

        info!("Wardoff started its named-pipe control server on {control_pipe_path}.");
        logger::log_event(
            "ipc_server_started",
            EventSource::Ipc,
            format!("Wardoff started its named-pipe control server on {control_pipe_path}."),
            true,
        );
        info!("Wardoff started its read-only status pipe on {status_pipe_path}.");
        logger::log_event(
            "status_server_started",
            EventSource::Ipc,
            format!("Wardoff started its read-only status pipe on {status_pipe_path}."),
            true,
        );

        Ok(Self {
            control_join_handle: Some(control_join_handle),
            status_join_handle: Some(status_join_handle),
            status_shutdown_requested,
        })
    }

    /// Stops accepting new status-pipe clients before the main runtime begins shutdown work.
    pub(crate) fn begin_shutdown(&self) {
        self.status_shutdown_requested
            .store(true, Ordering::Release);
        let _ = wake_status_server();
    }

    /// Stops the background named-pipe server and joins its worker thread.
    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        if self.control_join_handle.is_none() && self.status_join_handle.is_none() {
            return Ok(());
        }

        self.begin_shutdown();
        match send_request(&IpcRequest::ShutdownServer) {
            Ok(IpcResponse::Ok) | Err(ClientError::Unavailable) => {}
            Ok(IpcResponse::Error { message }) => {
                return Err(format!(
                    "Wardoff could not stop its IPC server cleanly: {message}"
                ));
            }
            Ok(IpcResponse::Status { .. }) => {
                return Err(
                    "Wardoff received an unexpected status payload while stopping its IPC server."
                        .to_string(),
                );
            }
            Err(ClientError::Transport(message)) => return Err(message),
        }

        if let Some(join_handle) = self.control_join_handle.take() {
            join_handle
                .join()
                .map_err(|_| "Wardoff IPC server thread panicked during shutdown.".to_string())?;
        }
        if let Some(join_handle) = self.status_join_handle.take() {
            join_handle.join().map_err(|_| {
                "Wardoff status server thread panicked during shutdown.".to_string()
            })?;
        }

        Ok(())
    }
}

/// Sends a JSON control request to the primary Wardoff runtime.
pub(crate) fn send_request(request: &IpcRequest) -> Result<IpcResponse, ClientError> {
    let control_pipe_path = current_control_pipe_path().map_err(ClientError::Transport)?;
    let mut pipe = connect_control_pipe(&control_pipe_path)?;
    let payload = serde_json::to_string(request).map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not serialize its IPC request: {error}"
        ))
    })?;

    pipe.write_all(payload.as_bytes()).map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not write to {control_pipe_path}: {error}"
        ))
    })?;
    pipe.write_all(b"\n").map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not finish writing to {control_pipe_path}: {error}"
        ))
    })?;
    pipe.flush().map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not flush {control_pipe_path}: {error}"
        ))
    })?;

    let mut response_line = String::new();
    let bytes_read = {
        let mut reader = BufReader::new(&mut pipe);
        reader.read_line(&mut response_line).map_err(|error| {
            ClientError::Transport(format!(
                "Wardoff could not read the reply from {control_pipe_path}: {error}"
            ))
        })?
    };

    if bytes_read == 0 {
        return Err(ClientError::Transport(format!(
            "Wardoff did not receive any reply from {control_pipe_path}."
        )));
    }

    serde_json::from_str(response_line.trim_end()).map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff received malformed IPC JSON from {control_pipe_path}: {error}"
        ))
    })
}

/// Reads the current runtime status through the dedicated read-only status pipe.
pub(crate) fn read_status() -> Result<StatusOutput, ClientError> {
    let status_pipe_path = current_status_pipe_path().map_err(ClientError::Transport)?;
    let mut pipe = connect_status_pipe(&status_pipe_path)?;
    let mut status_line = String::new();
    let bytes_read = {
        let mut reader = BufReader::new(&mut pipe);
        reader.read_line(&mut status_line).map_err(|error| {
            ClientError::Transport(format!(
                "Wardoff could not read the status reply from {status_pipe_path}: {error}"
            ))
        })?
    };

    if bytes_read == 0 {
        return Err(ClientError::Transport(format!(
            "Wardoff did not receive any status reply from {status_pipe_path}."
        )));
    }

    serde_json::from_str(status_line.trim_end()).map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff received malformed status JSON from {status_pipe_path}: {error}"
        ))
    })
}

impl From<PipeMode> for BlockerMode {
    fn from(mode: PipeMode) -> Self {
        match mode {
            PipeMode::Block => BlockerMode::Block,
            PipeMode::Allow => BlockerMode::Allow,
        }
    }
}

struct PipeHandle(HANDLE);

impl PipeHandle {
    fn into_file(mut self) -> File {
        let handle = self.0;
        self.0 = HANDLE::default();
        unsafe { File::from_raw_handle(handle.0 as RawHandle) }
    }
}

impl Drop for PipeHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct LocalSecurityDescriptor(*mut core::ffi::c_void);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct ControlPipeSecurityAttributes {
    security_attributes: SECURITY_ATTRIBUTES,
    _security_descriptor: LocalSecurityDescriptor,
}

fn run_server_loop(request_tx: Sender<PendingRequest>, ui_thread_id: u32) {
    let control_pipe_path = match current_control_pipe_path() {
        Ok(path) => path,
        Err(error) => {
            warn!("Wardoff stopped its named-pipe server before startup: {error}");
            logger::log_event(
                "ipc_server_stopped",
                EventSource::Ipc,
                format!("Wardoff stopped its named-pipe control server before startup: {error}"),
                false,
            );
            return;
        }
    };

    let control_pipe_security = match build_control_pipe_security_attributes(&control_pipe_path) {
        Ok(security) => security,
        Err(error) => {
            warn!("Wardoff stopped its named-pipe server before startup: {error}");
            logger::log_event(
                "ipc_server_stopped",
                EventSource::Ipc,
                format!("Wardoff stopped its named-pipe control server before startup: {error}"),
                false,
            );
            return;
        }
    };

    loop {
        let mut pipe = match accept_control_client(&control_pipe_security, &control_pipe_path) {
            Ok(pipe) => pipe,
            Err(error) => {
                warn!("Wardoff stopped its named-pipe server after an IPC error: {error}");
                logger::log_event(
                    "ipc_server_stopped",
                    EventSource::Ipc,
                    format!(
                        "Wardoff stopped its named-pipe control server after an IPC error: {error}"
                    ),
                    false,
                );
                break;
            }
        };

        match read_request(&mut pipe) {
            Ok(IpcRequest::ShutdownServer) => {
                let _ = write_response(&mut pipe, &IpcResponse::Ok);
                break;
            }
            Ok(request) => {
                let (response_tx, response_rx) = mpsc::channel();
                if request_tx
                    .send(PendingRequest {
                        request,
                        response_tx,
                    })
                    .is_err()
                {
                    let _ = write_response(
                        &mut pipe,
                        &IpcResponse::Error {
                            message: "Wardoff is shutting down.".to_string(),
                        },
                    );
                    break;
                }

                if let Err(error) = wake_ui_thread(ui_thread_id) {
                    let _ = write_response(
                        &mut pipe,
                        &IpcResponse::Error {
                            message: error.clone(),
                        },
                    );
                    warn!("{error}");
                    logger::log_event(
                        "ipc_request_rejected",
                        EventSource::Ipc,
                        format!("Wardoff could not wake its UI thread for IPC: {error}"),
                        false,
                    );
                    continue;
                }

                match response_rx.recv() {
                    Ok(response) => {
                        if let Err(error) = write_response(&mut pipe, &response) {
                            warn!("Wardoff could not send an IPC reply: {error}");
                        }
                    }
                    Err(_) => {
                        let _ = write_response(
                            &mut pipe,
                            &IpcResponse::Error {
                                message: "Wardoff is shutting down.".to_string(),
                            },
                        );
                        break;
                    }
                }
            }
            Err(error) => {
                let _ = write_response(
                    &mut pipe,
                    &IpcResponse::Error {
                        message: error.clone(),
                    },
                );
                warn!("Wardoff rejected an IPC request: {error}");
                logger::log_event(
                    "ipc_request_rejected",
                    EventSource::Ipc,
                    format!("Wardoff rejected an IPC request: {error}"),
                    false,
                );
            }
        }
    }

    info!("Wardoff stopped its named-pipe control server.");
    logger::log_event(
        "ipc_server_stopped",
        EventSource::Ipc,
        format!("Wardoff stopped its named-pipe control server on {control_pipe_path}."),
        true,
    );
}

fn run_status_server_loop(
    request_tx: Sender<PendingRequest>,
    ui_thread_id: u32,
    shutdown_requested: Arc<AtomicBool>,
) {
    let status_pipe_path = match current_status_pipe_path() {
        Ok(path) => path,
        Err(error) => {
            warn!("Wardoff stopped its status server before startup: {error}");
            logger::log_event(
                "status_server_stopped",
                EventSource::Ipc,
                format!("Wardoff stopped its read-only status pipe before startup: {error}"),
                false,
            );
            return;
        }
    };

    loop {
        if shutdown_requested.load(Ordering::Acquire) {
            break;
        }

        let mut pipe = match accept_status_client(&status_pipe_path) {
            Ok(pipe) => pipe,
            Err(error) => {
                warn!("Wardoff stopped its status server after an IPC error: {error}");
                logger::log_event(
                    "status_server_stopped",
                    EventSource::Ipc,
                    format!(
                        "Wardoff stopped its read-only status pipe after an IPC error: {error}"
                    ),
                    false,
                );
                break;
            }
        };

        if shutdown_requested.load(Ordering::Acquire) {
            break;
        }

        let (response_tx, response_rx) = mpsc::channel();
        if request_tx
            .send(PendingRequest {
                request: IpcRequest::Status,
                response_tx,
            })
            .is_err()
        {
            break;
        }

        if let Err(error) = wake_ui_thread(ui_thread_id) {
            warn!("{error}");
            logger::log_event(
                "status_request_rejected",
                EventSource::Ipc,
                format!("Wardoff could not wake its UI thread for a status request: {error}"),
                false,
            );
            continue;
        }

        match response_rx.recv() {
            Ok(IpcResponse::Status { status }) => {
                if let Err(error) = write_status_output(&mut pipe, &status) {
                    warn!("Wardoff could not send a status reply: {error}");
                }
            }
            Ok(IpcResponse::Error { message }) => {
                warn!("Wardoff could not produce a status reply: {message}");
                logger::log_event(
                    "status_request_rejected",
                    EventSource::Ipc,
                    format!("Wardoff could not produce a status reply: {message}"),
                    false,
                );
            }
            Ok(IpcResponse::Ok) => {
                warn!("Wardoff produced an empty reply for its read-only status pipe.");
                logger::log_event(
                    "status_request_rejected",
                    EventSource::Ipc,
                    "Wardoff produced an empty reply for its read-only status pipe.",
                    false,
                );
            }
            Err(_) => break,
        }
    }

    info!("Wardoff stopped its read-only status pipe.");
    logger::log_event(
        "status_server_stopped",
        EventSource::Ipc,
        format!("Wardoff stopped its read-only status pipe on {status_pipe_path}."),
        true,
    );
}

fn accept_control_client(
    security: &ControlPipeSecurityAttributes,
    control_pipe_path: &str,
) -> Result<File, String> {
    let control_pipe_path_wide = wide_null(control_pipe_path);
    let pipe = unsafe {
        CreateNamedPipeW(
            PCWSTR(control_pipe_path_wide.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            Some(&security.security_attributes),
        )
    };
    if pipe.is_invalid() {
        return Err(format!(
            "Wardoff could not create {control_pipe_path}: {}",
            WindowsError::from_thread()
        ));
    }

    let pipe = PipeHandle(pipe);
    match unsafe { ConnectNamedPipe(pipe.0, None) } {
        Ok(()) => {}
        Err(_) if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED => {}
        Err(error) => {
            return Err(format!(
                "Wardoff could not accept a client on {control_pipe_path}: {error}"
            ));
        }
    }

    Ok(pipe.into_file())
}

fn accept_status_client(status_pipe_path: &str) -> Result<File, String> {
    let status_pipe_path_wide = wide_null(status_pipe_path);
    let pipe = unsafe {
        CreateNamedPipeW(
            PCWSTR(status_pipe_path_wide.as_ptr()),
            PIPE_ACCESS_OUTBOUND,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            None,
        )
    };
    if pipe.is_invalid() {
        return Err(format!(
            "Wardoff could not create {status_pipe_path}: {}",
            WindowsError::from_thread()
        ));
    }

    let pipe = PipeHandle(pipe);
    match unsafe { ConnectNamedPipe(pipe.0, None) } {
        Ok(()) => {}
        Err(_) if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED => {}
        Err(error) => {
            return Err(format!(
                "Wardoff could not accept a client on {status_pipe_path}: {error}"
            ));
        }
    }

    Ok(pipe.into_file())
}

fn read_request(pipe: &mut File) -> Result<IpcRequest, String> {
    let mut request_line = String::new();
    let bytes_read = {
        let mut reader = BufReader::new(pipe);
        reader
            .read_line(&mut request_line)
            .map_err(|error| format!("Wardoff could not read an IPC request: {error}"))?
    };

    if bytes_read == 0 {
        return Err("Wardoff received an empty IPC request.".to_string());
    }

    serde_json::from_str(request_line.trim_end())
        .map_err(|error| format!("Wardoff received malformed IPC JSON: {error}"))
}

fn write_response(pipe: &mut File, response: &IpcResponse) -> Result<(), String> {
    let payload = serde_json::to_string(response)
        .map_err(|error| format!("Wardoff could not serialize an IPC reply: {error}"))?;

    pipe.write_all(payload.as_bytes())
        .map_err(|error| format!("Wardoff could not write an IPC reply: {error}"))?;
    pipe.write_all(b"\n")
        .map_err(|error| format!("Wardoff could not finish its IPC reply: {error}"))?;
    pipe.flush()
        .map_err(|error| format!("Wardoff could not flush its IPC reply: {error}"))
}

fn write_status_output(pipe: &mut File, status: &StatusOutput) -> Result<(), String> {
    let payload = status
        .to_json()
        .map_err(|error| format!("Wardoff could not serialize a status reply: {error}"))?;

    pipe.write_all(payload.as_bytes())
        .map_err(|error| format!("Wardoff could not write a status reply: {error}"))?;
    pipe.write_all(b"\n")
        .map_err(|error| format!("Wardoff could not finish its status reply: {error}"))?;
    pipe.flush()
        .map_err(|error| format!("Wardoff could not flush its status reply: {error}"))
}

fn connect_control_pipe(control_pipe_path: &str) -> Result<File, ClientError> {
    let mut last_error_code = None;

    for attempt in 0..PIPE_CONNECT_ATTEMPTS {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .open(control_pipe_path)
        {
            Ok(pipe) => return Ok(pipe),
            Err(error) => {
                let raw_code = error.raw_os_error();
                last_error_code = raw_code;

                if matches!(
                    raw_code,
                    Some(code)
                        if code == ERROR_FILE_NOT_FOUND.0 as i32
                            || code == ERROR_PATH_NOT_FOUND.0 as i32
                            || code == ERROR_PIPE_BUSY.0 as i32
                ) && attempt + 1 < PIPE_CONNECT_ATTEMPTS
                {
                    thread::sleep(PIPE_CONNECT_DELAY);
                    continue;
                }

                return Err(map_connect_error(control_pipe_path, true, error, raw_code));
            }
        }
    }

    final_connect_error(control_pipe_path, last_error_code)
}

fn connect_status_pipe(status_pipe_path: &str) -> Result<File, ClientError> {
    let mut last_error_code = None;

    for attempt in 0..PIPE_CONNECT_ATTEMPTS {
        match OpenOptions::new().read(true).open(status_pipe_path) {
            Ok(pipe) => return Ok(pipe),
            Err(error) => {
                let raw_code = error.raw_os_error();
                last_error_code = raw_code;

                if matches!(
                    raw_code,
                    Some(code)
                        if code == ERROR_FILE_NOT_FOUND.0 as i32
                            || code == ERROR_PATH_NOT_FOUND.0 as i32
                            || code == ERROR_PIPE_BUSY.0 as i32
                ) && attempt + 1 < PIPE_CONNECT_ATTEMPTS
                {
                    thread::sleep(PIPE_CONNECT_DELAY);
                    continue;
                }

                return Err(map_connect_error(status_pipe_path, false, error, raw_code));
            }
        }
    }

    final_connect_error(status_pipe_path, last_error_code)
}

fn final_connect_error(path: &str, last_error_code: Option<i32>) -> Result<File, ClientError> {
    match last_error_code {
        Some(code)
            if code == ERROR_FILE_NOT_FOUND.0 as i32 || code == ERROR_PATH_NOT_FOUND.0 as i32 =>
        {
            Err(ClientError::Unavailable)
        }
        Some(code) if code == ERROR_PIPE_BUSY.0 as i32 => Err(ClientError::Transport(format!(
            "Wardoff found a primary instance, but {path} stayed busy."
        ))),
        _ => Err(ClientError::Transport(format!(
            "Wardoff could not connect to {path}."
        ))),
    }
}

fn map_connect_error(
    path: &str,
    is_control_pipe: bool,
    error: std::io::Error,
    raw_code: Option<i32>,
) -> ClientError {
    match raw_code {
        Some(code)
            if code == ERROR_FILE_NOT_FOUND.0 as i32 || code == ERROR_PATH_NOT_FOUND.0 as i32 =>
        {
            ClientError::Unavailable
        }
        Some(code) if code == ERROR_ACCESS_DENIED.0 as i32 && is_control_pipe => {
            ClientError::Transport(control_pipe_access_denied_message())
        }
        Some(code) if code == ERROR_PIPE_BUSY.0 as i32 => ClientError::Transport(format!(
            "Wardoff found a primary instance, but {path} is busy: {error}"
        )),
        _ => ClientError::Transport(format!("Wardoff could not connect to {path}: {error}")),
    }
}

fn build_control_pipe_security_attributes(
    control_pipe_path: &str,
) -> Result<ControlPipeSecurityAttributes, String> {
    let current_user_sid = current_process_user_sid_string(control_pipe_path)?;
    let control_pipe_security_sddl = wide_null(&build_control_pipe_security_sddl(
        &current_user_sid,
        control_pipe_path,
    )?);
    let mut security_descriptor = PSECURITY_DESCRIPTOR::default();

    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(control_pipe_security_sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut security_descriptor,
            None,
        )
        .map_err(|error| {
            format!("Wardoff could not prepare {control_pipe_path} security attributes: {error}")
        })?;
    }

    Ok(ControlPipeSecurityAttributes {
        security_attributes: SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security_descriptor.0,
            bInheritHandle: false.into(),
        },
        _security_descriptor: LocalSecurityDescriptor(security_descriptor.0),
    })
}

fn build_control_pipe_security_sddl(
    current_user_sid: &str,
    control_pipe_path: &str,
) -> Result<String, String> {
    if current_user_sid.trim().is_empty() {
        return Err(format!(
            "Wardoff could not build {control_pipe_path} security attributes because the current-user SID was empty."
        ));
    }

    Ok(format!(
        "{CONTROL_PIPE_SECURITY_SDDL_PREFIX}{current_user_sid}{CONTROL_PIPE_SECURITY_SDDL_SUFFIX}"
    ))
}

fn control_pipe_access_denied_message() -> String {
    "Wardoff could not send that control command to the active primary runtime. Non-elevated shells can control an elevated Wardoff runtime only for the same interactive Windows user. If Wardoff is running as a different user, close that runtime or relaunch the command from the matching account.".to_string()
}

fn current_process_user_sid_string(control_pipe_path: &str) -> Result<String, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(|error| {
            format!("Wardoff could not query its process token for {control_pipe_path}: {error}")
        })?;
        let token = HandleGuard(token);

        let mut required_length = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut required_length);
        let probe_error = GetLastError();
        if required_length == 0 || probe_error != ERROR_INSUFFICIENT_BUFFER {
            return Err(format!(
                "Wardoff could not determine the token-user size for {control_pipe_path} (Win32 error {}).",
                probe_error.0
            ));
        }

        let mut token_information = vec![0u8; required_length as usize];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(token_information.as_mut_ptr() as *mut _),
            required_length,
            &mut required_length,
        )
        .map_err(|error| {
            format!(
                "Wardoff could not read its process token user for {control_pipe_path}: {error}"
            )
        })?;

        let token_user = &*(token_information.as_ptr() as *const TOKEN_USER);
        if token_user.User.Sid.0.is_null() {
            return Err(format!(
                "Wardoff could not read a token-user SID for {control_pipe_path}."
            ));
        }
        let mut string_sid = PWSTR::null();
        ConvertSidToStringSidW(token_user.User.Sid, &mut string_sid).map_err(|error| {
            format!(
                "Wardoff could not stringify its token-user SID for {control_pipe_path}: {error}"
            )
        })?;

        let string_sid = LocalAllocatedWideString(string_sid);
        let sid = string_sid.to_string();
        if sid.is_empty() {
            return Err(format!(
                "Wardoff could not stringify a non-empty token-user SID for {control_pipe_path}."
            ));
        }

        Ok(sid)
    }
}

fn wake_status_server() -> std::io::Result<()> {
    let status_pipe_path =
        current_status_pipe_path().map_err(|error| std::io::Error::other(error))?;
    OpenOptions::new()
        .read(true)
        .open(status_pipe_path)
        .map(|_| ())
}

fn wake_ui_thread(ui_thread_id: u32) -> Result<(), String> {
    unsafe { PostThreadMessageW(ui_thread_id, IPC_WAKE_MESSAGE, WPARAM(0), LPARAM(0)) }
        .map_err(|error| format!("Wardoff could not wake its UI thread for IPC: {error}"))
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

struct LocalAllocatedWideString(PWSTR);

impl std::fmt::Display for LocalAllocatedWideString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_null() {
            return Ok(());
        }

        let s = unsafe {
            let mut length = 0usize;
            while *self.0 .0.add(length) != 0 {
                length += 1;
            }

            String::from_utf16_lossy(std::slice::from_raw_parts(self.0 .0, length))
        };

        f.write_str(&s)
    }
}

impl Drop for LocalAllocatedWideString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0 .0 as *mut _)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_pipe_access_denied_maps_to_product_message() {
        let error = std::io::Error::from_raw_os_error(ERROR_ACCESS_DENIED.0 as i32);

        match map_connect_error(
            r"\\.\pipe\WardoffControl-Session-7",
            true,
            error,
            Some(ERROR_ACCESS_DENIED.0 as i32),
        ) {
            ClientError::Transport(message) => {
                assert_eq!(message, control_pipe_access_denied_message());
            }
            ClientError::Unavailable => panic!("expected a transport error"),
        }
    }

    #[test]
    fn control_pipe_security_descriptor_stays_same_user_and_medium_integrity_only() {
        let sddl = build_control_pipe_security_sddl(
            "S-1-5-21-123-456-789-1001",
            r"\\.\pipe\WardoffControl-Session-7",
        )
        .expect("expected a valid same-user control-pipe SDDL");

        assert_eq!(
            sddl,
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;S-1-5-21-123-456-789-1001)S:(ML;;NW;;;ME)"
        );
        assert!(!sddl.contains("OW"));
    }

    #[test]
    fn control_pipe_security_descriptor_rejects_empty_sid() {
        let error = build_control_pipe_security_sddl("", r"\\.\pipe\WardoffControl-Session-7")
            .expect_err("expected an empty SID to be rejected");

        assert!(error.contains("current-user SID was empty"));
    }
}
