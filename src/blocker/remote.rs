use crate::logger::{self, EventSource};
use log::{error, info, warn};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, Sender},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::{Error as WindowsError, Result as WindowsResult, HRESULT};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_NO_SHUTDOWN_IN_PROGRESS, E_ACCESSDENIED,
};
use windows::Win32::System::Shutdown::AbortSystemShutdownW;

const LAYER4_THREAD_NAME: &str = "wardoff-layer4-remote-shutdown";

/// Coordinates Layer 4 remote shutdown abort polling.
pub struct RemoteShutdownBlocker {
    worker: Option<RemoteShutdownWorker>,
    active_state: Arc<AtomicBool>,
}

impl RemoteShutdownBlocker {
    /// Starts Layer 4 remote-shutdown polling if it is not already active.
    pub fn activate(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return Ok(());
        }

        let (stop_tx, stop_rx) = mpsc::channel();
        let active_state = Arc::clone(&self.active_state);

        match thread::Builder::new()
            .name(LAYER4_THREAD_NAME.to_string())
            .spawn(move || run_worker(active_state, stop_rx))
        {
            Ok(join_handle) => {
                self.worker = Some(RemoteShutdownWorker {
                    stop_tx,
                    join_handle: Some(join_handle),
                });
                Ok(())
            }
            Err(error) => Err(format!(
                "Layer 4 could not start its polling thread: {error}"
            )),
        }
    }

    /// Stops Layer 4 polling when Block mode ends.
    pub fn deactivate(&mut self) {
        let Some(mut worker) = self.worker.take() else {
            return;
        };

        self.active_state.store(false, Ordering::Release);
        let _ = worker.stop_tx.send(());

        if let Some(join_handle) = worker.join_handle.take() {
            if join_handle.join().is_err() {
                error!("Layer 4 worker thread panicked while stopping");
                logger::log_event(
                    "remote_layer_disabled",
                    EventSource::Remote,
                    "Layer 4 worker thread panicked while stopping.",
                    false,
                );
            }
        }
    }

    /// Returns whether Layer 4 is currently polling for remote shutdowns.
    pub fn is_active(&self) -> bool {
        self.active_state.load(Ordering::Acquire)
    }
}

impl Drop for RemoteShutdownBlocker {
    fn drop(&mut self) {
        self.deactivate();
    }
}

impl Default for RemoteShutdownBlocker {
    fn default() -> Self {
        Self {
            worker: None,
            active_state: Arc::new(AtomicBool::new(false)),
        }
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

struct ActiveStateGuard(Arc<AtomicBool>);

impl ActiveStateGuard {
    fn activate(active_state: Arc<AtomicBool>) -> Self {
        active_state.store(true, Ordering::Release);
        Self(active_state)
    }
}

impl Drop for ActiveStateGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn run_worker(active_state: Arc<AtomicBool>, stop_rx: Receiver<()>) {
    if !poll_remote_shutdown() {
        return;
    }

    let _active_guard = ActiveStateGuard::activate(active_state);
    info!(
        "Layer 4 remote shutdown polling is active and will call AbortSystemShutdownW(None) every {} ms while Block mode is active.",
        remote_abort_interval().as_millis()
    );
    logger::log_event(
        "remote_layer_enabled",
        EventSource::Remote,
        format!(
            "Layer 4 started polling AbortSystemShutdownW(None) every {} ms.",
            remote_abort_interval().as_millis()
        ),
        true,
    );
    let mut stopped_cleanly = true;

    loop {
        match stop_rx.recv_timeout(remote_abort_interval()) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if !poll_remote_shutdown() {
                    stopped_cleanly = false;
                    break;
                }
            }
        }
    }

    if stopped_cleanly {
        logger::log_event(
            "remote_layer_disabled",
            EventSource::Remote,
            "Layer 4 stopped remote shutdown polling as Block mode ended.",
            true,
        );
    }
}

fn poll_remote_shutdown() -> bool {
    match abort_remote_shutdown() {
        Ok(()) => {
            super::record_blocked_event();
            info!("Layer 4 intercepted and aborted a pending remote shutdown.");
            logger::log_event(
                "remote_shutdown_intercepted",
                EventSource::Remote,
                "Layer 4 called AbortSystemShutdownW(None) and aborted a pending remote shutdown.",
                true,
            );
            true
        }
        Err(error) if is_no_shutdown_in_progress_error(&error) => true,
        Err(error) if is_access_denied_error(&error) => {
            warn!(
                "Layer 4 could not call AbortSystemShutdownW(None) because this process lacks the required shutdown privilege; skipping Layer 4 remote shutdown protection: {error}"
            );
            logger::log_event(
                "remote_layer_enabled",
                EventSource::Remote,
                format!(
                    "Layer 4 could not call AbortSystemShutdownW(None) because the required shutdown privilege is missing: {error}"
                ),
                false,
            );
            false
        }
        Err(error) => {
            warn!(
                "Layer 4 polling stopped after AbortSystemShutdownW(None) failed unexpectedly: {error}"
            );
            logger::log_event(
                "remote_layer_disabled",
                EventSource::Remote,
                format!(
                    "Layer 4 polling stopped after AbortSystemShutdownW(None) failed unexpectedly: {error}"
                ),
                false,
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
