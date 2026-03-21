use log::{error, info, warn};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::{Error as WindowsError, Result as WindowsResult};
use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};

const SLEEP_BLOCKER_THREAD_NAME: &str = "wardoff-sleep-blocker";

/// Coordinates SetThreadExecutionState-based sleep and hibernate blocking.
#[derive(Default)]
pub struct SleepBlocker {
    worker: Option<SleepWorker>,
}

impl SleepBlocker {
    /// Starts the dedicated worker thread that owns the execution-state request.
    pub fn start_blocking() -> Self {
        let mut blocker = Self::default();
        if let Err(error) = blocker.activate() {
            warn!("{error}");
        }
        blocker
    }

    /// Starts sleep, hibernate, and display-idle blocking if it is not already active.
    pub fn activate(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return Ok(());
        }

        let (stop_tx, stop_rx) = mpsc::channel();

        match thread::Builder::new()
            .name(SLEEP_BLOCKER_THREAD_NAME.to_string())
            .spawn(move || run_worker(stop_rx))
        {
            Ok(join_handle) => {
                self.worker = Some(SleepWorker {
                    stop_tx,
                    join_handle: Some(join_handle),
                });
                Ok(())
            }
            Err(error) => Err(format!(
                "Sleep blocking could not start its worker thread: {error}"
            )),
        }
    }

    /// Stops the worker thread and clears the execution-state request on the same thread.
    pub fn deactivate(&mut self) {
        let Some(mut worker) = self.worker.take() else {
            return;
        };

        let _ = worker.stop_tx.send(());

        if let Some(join_handle) = worker.join_handle.take() {
            if join_handle.join().is_err() {
                error!("Sleep-blocking worker thread panicked while stopping");
            }
        }
    }

    /// Returns whether the sleep-blocking worker thread is currently active.
    pub fn is_active(&self) -> bool {
        self.worker.is_some()
    }
}

impl Drop for SleepBlocker {
    fn drop(&mut self) {
        self.deactivate();
    }
}

/// Enables the execution state requests that keep the system awake while blocking is active.
pub fn enable_sleep_block() -> WindowsResult<()> {
    let execution_state = ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED;

    if unsafe { SetThreadExecutionState(execution_state) }.0 == 0 {
        Err(WindowsError::from_thread())
    } else {
        Ok(())
    }
}

/// Disables the execution state requests that were used to keep the system awake.
pub fn disable_sleep_block() -> WindowsResult<()> {
    if unsafe { SetThreadExecutionState(ES_CONTINUOUS) }.0 == 0 {
        Err(WindowsError::from_thread())
    } else {
        Ok(())
    }
}

/// Returns the interval used to refresh the execution state request.
pub fn refresh_interval() -> Duration {
    Duration::from_secs(30)
}

struct SleepWorker {
    stop_tx: Sender<()>,
    join_handle: Option<JoinHandle<()>>,
}

fn run_worker(stop_rx: Receiver<()>) {
    if let Err(error) = enable_sleep_block() {
        warn!(
            "Sleep blocking could not activate SetThreadExecutionState; skipping sleep, hibernate, and display-idle protection: {error}"
        );
        return;
    }

    info!(
        "Sleep, hibernate, and display-idle blocking are active and will refresh SetThreadExecutionState every {} seconds while Block mode is active.",
        refresh_interval().as_secs()
    );

    loop {
        match stop_rx.recv_timeout(refresh_interval()) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if let Err(error) = enable_sleep_block() {
                    warn!(
                        "Sleep blocking stopped after its SetThreadExecutionState refresh failed: {error}"
                    );
                    break;
                }
            }
        }
    }

    if let Err(error) = disable_sleep_block() {
        warn!(
            "Sleep blocking could not clear its SetThreadExecutionState request during shutdown: {error}"
        );
    }
}
