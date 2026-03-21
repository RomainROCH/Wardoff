use log::{error, info, warn};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::{Error as WindowsError, Result as WindowsResult, HRESULT};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_NO_SHUTDOWN_IN_PROGRESS, E_ACCESSDENIED,
};
use windows::Win32::System::Shutdown::AbortSystemShutdownW;

const LAYER4_THREAD_NAME: &str = "wardoff-layer4-remote-shutdown";

/// Coordinates Layer 4 remote shutdown abort polling.
#[derive(Default)]
pub struct RemoteShutdownBlocker {
    worker: Option<RemoteShutdownWorker>,
}

impl RemoteShutdownBlocker {
    /// Starts Layer 4 remote-shutdown polling while Wardoff remains in Block mode.
    pub fn start_blocking() -> Self {
        let (stop_tx, stop_rx) = mpsc::channel();

        match thread::Builder::new()
            .name(LAYER4_THREAD_NAME.to_string())
            .spawn(move || run_worker(stop_rx))
        {
            Ok(join_handle) => Self {
                worker: Some(RemoteShutdownWorker {
                    stop_tx,
                    join_handle: Some(join_handle),
                }),
            },
            Err(error) => {
                warn!(
                    "Layer 4 could not start its polling thread: {error}. Skipping Layer 4 remote shutdown protection."
                );
                Self::default()
            }
        }
    }

    /// Stops Layer 4 polling when Block mode ends.
    pub fn deactivate(&mut self) {
        let Some(mut worker) = self.worker.take() else {
            return;
        };

        let _ = worker.stop_tx.send(());

        if let Some(join_handle) = worker.join_handle.take() {
            if join_handle.join().is_err() {
                error!("Layer 4 worker thread panicked while stopping");
            }
        }
    }
}

impl Drop for RemoteShutdownBlocker {
    fn drop(&mut self) {
        self.deactivate();
    }
}

/// Returns the polling interval used by the remote shutdown abort loop.
pub fn remote_abort_interval() -> Duration {
    Duration::from_millis(900)
}

/// Issues the Windows call used to cancel a pending remote shutdown.
pub fn abort_remote_shutdown() -> WindowsResult<()> {
    unsafe { AbortSystemShutdownW(None) }
}

struct RemoteShutdownWorker {
    stop_tx: Sender<()>,
    join_handle: Option<JoinHandle<()>>,
}

fn run_worker(stop_rx: Receiver<()>) {
    if !poll_remote_shutdown() {
        return;
    }

    info!(
        "Layer 4 remote shutdown polling is active and will call AbortSystemShutdownW(None) every {} ms while Block mode is active.",
        remote_abort_interval().as_millis()
    );

    loop {
        match stop_rx.recv_timeout(remote_abort_interval()) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if !poll_remote_shutdown() {
                    break;
                }
            }
        }
    }
}

fn poll_remote_shutdown() -> bool {
    match abort_remote_shutdown() {
        Ok(()) => {
            info!("Layer 4 intercepted and aborted a pending remote shutdown.");
            true
        }
        Err(error) if is_no_shutdown_in_progress_error(&error) => true,
        Err(error) if is_access_denied_error(&error) => {
            warn!(
                "Layer 4 could not call AbortSystemShutdownW(None) because this process lacks the required shutdown privilege; skipping Layer 4 remote shutdown protection: {error}"
            );
            false
        }
        Err(error) => {
            warn!(
                "Layer 4 polling stopped after AbortSystemShutdownW(None) failed unexpectedly: {error}"
            );
            false
        }
    }
}

fn is_access_denied_error(error: &WindowsError) -> bool {
    let code = error.code();
    code == E_ACCESSDENIED || code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
}

fn is_no_shutdown_in_progress_error(error: &WindowsError) -> bool {
    error.code() == HRESULT::from_win32(ERROR_NO_SHUTDOWN_IN_PROGRESS.0)
}
