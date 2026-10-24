use crate::blocker::abort::{abort_pending_shutdown, ShutdownAbortOutcome};
use crate::blocker::observation::SharedObservation;
use crate::diagnostics::Observation;
use crate::logger::{self, EventSource};
use log::{error, info, warn};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const LAYER4_THREAD_NAME: &str = "wardoff-layer4-remote-shutdown";

/// Coordinates Layer 4 remote shutdown abort polling.
#[derive(Default)]
pub struct RemoteShutdownBlocker {
    worker: Option<RemoteShutdownWorker>,
    active_state: SharedObservation,
}

impl RemoteShutdownBlocker {
    /// Starts Layer 4 remote-shutdown polling if it is not already active.
    pub fn activate(&mut self) -> Result<(), String> {
        self.cleanup_finished_worker();
        if self.worker.is_some() {
            return Ok(());
        }
        self.active_state.set(Observation::Starting);

        let (stop_tx, stop_rx) = mpsc::channel();
        let active_state = self.active_state.clone();

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
            Err(error) => {
                self.active_state.set(Observation::ActivationFailed);
                Err(format!(
                    "Layer 4 could not start its polling thread: {error}"
                ))
            }
        }
    }

    /// Stops Layer 4 polling when Block mode ends.
    pub fn deactivate(&mut self) {
        self.begin_forced_shutdown_cleanup();

        let Some(mut worker) = self.worker.take() else {
            return;
        };

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
        self.active_state.set(Observation::Inactive);
    }

    /// Requests Layer 4 shutdown polling to stop without blocking on thread join.
    pub(crate) fn begin_forced_shutdown_cleanup(&mut self) {
        let Some(worker) = self.worker.take() else {
            self.active_state.set(Observation::Inactive);
            return;
        };
        self.active_state.set(Observation::Inactive);
        let _ = worker.stop_tx.send(());
        self.worker = Some(worker);
    }

    /// Returns whether Layer 4 is currently polling for remote shutdowns.
    pub(crate) fn observation(&self) -> Observation {
        self.active_state.get()
    }

    fn cleanup_finished_worker(&mut self) {
        let should_join = self
            .worker
            .as_ref()
            .and_then(|worker| worker.join_handle.as_ref())
            .is_some_and(JoinHandle::is_finished);
        if !should_join {
            return;
        }
        if let Some(mut worker) = self.worker.take() {
            if let Some(join_handle) = worker.join_handle.take() {
                let _ = join_handle.join();
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

struct RemoteShutdownWorker {
    stop_tx: Sender<()>,
    join_handle: Option<JoinHandle<()>>,
}

struct ActiveStateGuard(SharedObservation);

impl ActiveStateGuard {
    fn new(active_state: SharedObservation) -> Self {
        Self(active_state)
    }
}

impl Drop for ActiveStateGuard {
    fn drop(&mut self) {
        self.0.finish_worker(Observation::MonitoringFailed);
    }
}

fn run_worker(active_state: SharedObservation, stop_rx: Receiver<()>) {
    let _active_guard = ActiveStateGuard::new(active_state.clone());
    if let Err(observation) = poll_remote_shutdown_observation() {
        active_state.set(match observation {
            Observation::MonitoringFailed => Observation::ActivationFailed,
            other => other,
        });
        return;
    }

    active_state.set(Observation::Active);
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
                if let Err(observation) = poll_remote_shutdown_observation() {
                    active_state.set(observation);
                    stopped_cleanly = false;
                    break;
                }
            }
        }
    }

    if stopped_cleanly {
        active_state.set(Observation::Inactive);
        logger::log_event(
            "remote_layer_disabled",
            EventSource::Remote,
            "Layer 4 stopped remote shutdown polling as Block mode ended.",
            true,
        );
    }
}

fn poll_remote_shutdown_observation() -> Result<(), Observation> {
    match abort_pending_shutdown() {
        ShutdownAbortOutcome::Aborted => {
            super::record_blocked_event();
            info!("Layer 4 intercepted and aborted a pending remote shutdown.");
            logger::log_event(
                "remote_shutdown_intercepted",
                EventSource::Remote,
                "Layer 4 called AbortSystemShutdownW(None) and aborted a pending remote shutdown.",
                true,
            );
            Ok(())
        }
        ShutdownAbortOutcome::NoShutdownPending => Ok(()),
        ShutdownAbortOutcome::AccessDenied { details } => {
            warn!(
                "Layer 4 could not call AbortSystemShutdownW(None) because this process lacks the required shutdown privilege; skipping Layer 4 remote shutdown protection: {details}"
            );
            logger::log_event(
                "remote_layer_enabled",
                EventSource::Remote,
                format!(
                    "Layer 4 could not call AbortSystemShutdownW(None) because the required shutdown privilege is missing: {details}"
                ),
                false,
            );
            Err(Observation::ElevationRequired)
        }
        ShutdownAbortOutcome::Failed { details } => {
            warn!(
                "Layer 4 polling stopped after AbortSystemShutdownW(None) failed unexpectedly: {details}"
            );
            logger::log_event(
                "remote_layer_disabled",
                EventSource::Remote,
                format!(
                    "Layer 4 polling stopped after AbortSystemShutdownW(None) failed unexpectedly: {details}"
                ),
                false,
            );
            Err(Observation::MonitoringFailed)
        }
    }
}
