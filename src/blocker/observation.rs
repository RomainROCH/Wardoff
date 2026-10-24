use crate::diagnostics::Observation;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Shared cached state for an asynchronous protection layer.
///
/// The state is deliberately separate from worker ownership: a worker can
/// finish after its owner has begun a retry, but it may only replace a
/// currently active/starting state.  Owners join the previous worker before a
/// retry, which provides the generation boundary; this CAS preserves failure
/// causes from the current worker during ordinary teardown.
#[derive(Clone, Debug)]
pub(crate) struct SharedObservation {
    state: Arc<AtomicU8>,
}

impl Default for SharedObservation {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedObservation {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(Observation::Inactive as u8)),
        }
    }

    pub(crate) fn get(&self) -> Observation {
        match self.state.load(Ordering::Acquire) {
            0 => Observation::Inactive,
            1 => Observation::Starting,
            2 => Observation::Active,
            3 => Observation::ElevationRequired,
            4 => Observation::ElevationUnknown,
            5 => Observation::TaskAbsent,
            6 => Observation::ActivationFailed,
            7 => Observation::MonitoringFailed,
            _ => Observation::Unknown,
        }
    }

    pub(crate) fn set(&self, observation: Observation) {
        self.state.store(observation as u8, Ordering::Release);
    }

    /// Completes a worker generation without overwriting a recorded failure.
    pub(crate) fn finish_worker(&self, replacement: Observation) {
        let _ = self.state.compare_exchange(
            Observation::Active as u8,
            replacement as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        let _ = self.state.compare_exchange(
            Observation::Starting as u8,
            replacement as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_finish_does_not_erase_a_recorded_failure() {
        let observation = SharedObservation::default();
        observation.set(Observation::Active);
        observation.set(Observation::MonitoringFailed);
        observation.finish_worker(Observation::Inactive);
        assert_eq!(observation.get(), Observation::MonitoringFailed);
    }

    #[test]
    fn unexpected_worker_exit_is_a_monitoring_failure() {
        let observation = SharedObservation::default();
        observation.set(Observation::Active);
        observation.finish_worker(Observation::MonitoringFailed);
        assert_eq!(observation.get(), Observation::MonitoringFailed);
    }

    #[test]
    fn explicit_stop_clears_a_failure_and_retry_enters_starting() {
        let observation = SharedObservation::default();
        observation.set(Observation::ActivationFailed);
        observation.set(Observation::Inactive);
        assert_eq!(observation.get(), Observation::Inactive);
        observation.set(Observation::Starting);
        assert_eq!(observation.get(), Observation::Starting);
    }

    #[test]
    fn active_is_reserved_for_a_ready_worker() {
        let observation = SharedObservation::default();
        observation.set(Observation::Starting);
        assert_ne!(observation.get(), Observation::Active);
        observation.set(Observation::Active);
        assert_eq!(observation.get(), Observation::Active);
    }

    #[test]
    fn explicit_normal_exit_can_clear_active_state() {
        let observation = SharedObservation::default();
        observation.set(Observation::Active);
        observation.finish_worker(Observation::Inactive);
        assert_eq!(observation.get(), Observation::Inactive);
    }
}
