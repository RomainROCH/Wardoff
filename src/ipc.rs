use crate::blocker::BlockerMode;
use crate::cli::StatusOutput;
use crate::logger::{self, EventSource};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::{w, Error as WindowsError};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};

const CONTROL_PIPE_PATH: &str = r"\\.\pipe\WardoffControl";
const IPC_SERVER_THREAD_NAME: &str = "wardoff-ipc-server";
const PIPE_BUFFER_SIZE: u32 = 4096;
const PIPE_CONNECT_ATTEMPTS: usize = 20;
const PIPE_CONNECT_DELAY: Duration = Duration::from_millis(100);

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
    join_handle: Option<JoinHandle<()>>,
}

impl IpcServer {
    /// Starts the background named-pipe server for the primary runtime.
    pub(crate) fn start(
        request_tx: Sender<PendingRequest>,
        ui_thread_id: u32,
    ) -> Result<Self, String> {
        let join_handle = thread::Builder::new()
            .name(IPC_SERVER_THREAD_NAME.to_string())
            .spawn(move || run_server_loop(request_tx, ui_thread_id))
            .map_err(|error| format!("Wardoff could not start its IPC server thread: {error}"))?;

        info!("Wardoff started its named-pipe control server on {CONTROL_PIPE_PATH}.");
        logger::log_event(
            "ipc_server_started",
            EventSource::Ipc,
            format!("Wardoff started its named-pipe control server on {CONTROL_PIPE_PATH}."),
            true,
        );

        Ok(Self {
            join_handle: Some(join_handle),
        })
    }

    /// Stops the background named-pipe server and joins its worker thread.
    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        if self.join_handle.is_none() {
            return Ok(());
        }

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

        if let Some(join_handle) = self.join_handle.take() {
            join_handle
                .join()
                .map_err(|_| "Wardoff IPC server thread panicked during shutdown.".to_string())?;
        }

        Ok(())
    }
}

/// Sends a JSON control request to the primary Wardoff runtime.
pub(crate) fn send_request(request: &IpcRequest) -> Result<IpcResponse, ClientError> {
    let mut pipe = connect_client_pipe()?;
    let payload = serde_json::to_string(request).map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not serialize its IPC request: {error}"
        ))
    })?;

    pipe.write_all(payload.as_bytes()).map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not write to {CONTROL_PIPE_PATH}: {error}"
        ))
    })?;
    pipe.write_all(b"\n").map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not finish writing to {CONTROL_PIPE_PATH}: {error}"
        ))
    })?;
    pipe.flush().map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff could not flush {CONTROL_PIPE_PATH}: {error}"
        ))
    })?;

    let mut response_line = String::new();
    let bytes_read = {
        let mut reader = BufReader::new(&mut pipe);
        reader.read_line(&mut response_line).map_err(|error| {
            ClientError::Transport(format!(
                "Wardoff could not read the reply from {CONTROL_PIPE_PATH}: {error}"
            ))
        })?
    };

    if bytes_read == 0 {
        return Err(ClientError::Transport(format!(
            "Wardoff did not receive any reply from {CONTROL_PIPE_PATH}."
        )));
    }

    serde_json::from_str(response_line.trim_end()).map_err(|error| {
        ClientError::Transport(format!(
            "Wardoff received malformed IPC JSON from {CONTROL_PIPE_PATH}: {error}"
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

fn run_server_loop(request_tx: Sender<PendingRequest>, ui_thread_id: u32) {
    loop {
        let mut pipe = match accept_client() {
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
        format!("Wardoff stopped its named-pipe control server on {CONTROL_PIPE_PATH}."),
        true,
    );
}

fn accept_client() -> Result<File, String> {
    let pipe = unsafe {
        CreateNamedPipeW(
            w!("\\\\.\\pipe\\WardoffControl"),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            1,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            None,
        )
    };
    if pipe.is_invalid() {
        return Err(format!(
            "Wardoff could not create {CONTROL_PIPE_PATH}: {}",
            WindowsError::from_thread()
        ));
    }

    let pipe = PipeHandle(pipe);
    match unsafe { ConnectNamedPipe(pipe.0, None) } {
        Ok(()) => {}
        Err(_) if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED => {}
        Err(error) => {
            return Err(format!(
                "Wardoff could not accept a client on {CONTROL_PIPE_PATH}: {error}"
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

fn connect_client_pipe() -> Result<File, ClientError> {
    let mut last_error_code = None;

    for attempt in 0..PIPE_CONNECT_ATTEMPTS {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .open(CONTROL_PIPE_PATH)
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

                return Err(map_connect_error(error, raw_code));
            }
        }
    }

    match last_error_code {
        Some(code)
            if code == ERROR_FILE_NOT_FOUND.0 as i32 || code == ERROR_PATH_NOT_FOUND.0 as i32 =>
        {
            Err(ClientError::Unavailable)
        }
        Some(code) if code == ERROR_PIPE_BUSY.0 as i32 => Err(ClientError::Transport(format!(
            "Wardoff found a primary instance, but {CONTROL_PIPE_PATH} stayed busy."
        ))),
        _ => Err(ClientError::Transport(format!(
            "Wardoff could not connect to {CONTROL_PIPE_PATH}."
        ))),
    }
}

fn map_connect_error(error: std::io::Error, raw_code: Option<i32>) -> ClientError {
    match raw_code {
        Some(code)
            if code == ERROR_FILE_NOT_FOUND.0 as i32 || code == ERROR_PATH_NOT_FOUND.0 as i32 =>
        {
            ClientError::Unavailable
        }
        Some(code) if code == ERROR_PIPE_BUSY.0 as i32 => ClientError::Transport(format!(
            "Wardoff found a primary instance, but {CONTROL_PIPE_PATH} is busy: {error}"
        )),
        _ => ClientError::Transport(format!(
            "Wardoff could not connect to {CONTROL_PIPE_PATH}: {error}"
        )),
    }
}

fn wake_ui_thread(ui_thread_id: u32) -> Result<(), String> {
    unsafe { PostThreadMessageW(ui_thread_id, IPC_WAKE_MESSAGE, WPARAM(0), LPARAM(0)) }
        .map_err(|error| format!("Wardoff could not wake its UI thread for IPC: {error}"))
}
