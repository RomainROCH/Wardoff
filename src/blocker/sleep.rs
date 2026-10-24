use crate::diagnostics::Observation;
use crate::logger::{self, EventSource};
use crate::power_intent::PowerIntent;
use log::{info, warn};
use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired,
    PowerSetRequest, POWER_REQUEST_TYPE,
};
use windows::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
use windows::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

const REQUEST_TYPES: [POWER_REQUEST_TYPE; 2] =
    [PowerRequestSystemRequired, PowerRequestDisplayRequired];

/// Owns idle-sleep/display-timeout requests, not a veto over explicit Sleep/Hibernate.
#[derive(Default)]
pub struct SleepBlocker {
    requests: PowerRequests<WindowsPowerApi>,
}

impl SleepBlocker {
    /// Acquires the selected requests, preserving the system request during flips.
    pub fn activate(&mut self, intent: PowerIntent) -> Result<(), String> {
        let was_active = self.is_active();
        let previous_mask = self.requests.acquired;
        if let Err(error) = self.requests.activate(intent) {
            warn!("Could not acquire idle-power requests: {error}");
            logger::log_event("sleep_blocked", EventSource::Sleep, &error, false);
            return Err(error);
        }
        if was_active && previous_mask == intent.request_mask() {
            return Ok(());
        }
        let message = match intent {
            PowerIntent::SystemOnly => {
                "Wardoff acquired its system Power Request for idle-sleep protection while allowing the display's normal inactivity timeout. Windows policy and explicit Sleep/Hibernate can override this protection."
            }
            PowerIntent::SystemAndDisplay => {
                "Wardoff acquired system/display Power Requests for idle-sleep and automatic display-timeout prevention. Windows policy and explicit Sleep/Hibernate can override this protection."
            }
        };
        info!("{message}");
        // Keep the existing event name for log consumers; success means request
        // acquisition, not that an actual sleep transition was observed or vetoed.
        logger::log_event("sleep_blocked", EventSource::Sleep, message, true);
        Ok(())
    }

    /// Releases the requests and closes their private process-owned handle.
    pub fn deactivate(&mut self) {
        if self.requests.handle.is_none() {
            self.requests.observation = Observation::Inactive;
            return;
        }
        match self.requests.release() {
            Ok(()) => logger::log_event(
                "sleep_unblocked",
                EventSource::Sleep,
                "Wardoff released its idle-power requests and closed the request handle.",
                true,
            ),
            Err(error) => {
                warn!("Idle-power request cleanup failed: {error}");
                logger::log_event("sleep_unblocked", EventSource::Sleep, error, false);
            }
        }
    }

    /// Releases requests immediately without a worker thread to join.
    pub(crate) fn begin_forced_shutdown_cleanup(&mut self) {
        self.deactivate();
    }

    /// Renews requests after the existing runtime resume notification.
    pub(crate) fn renew_after_resume(&mut self, intent: PowerIntent) -> Result<(), String> {
        // PowerSetRequest documents termination at user-initiated sleep entry:
        // https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-powersetrequest
        // The handle need not be invalid. Close the old object instead of
        // incrementing possibly surviving counts or clearing already-ended ones.
        // Tray power actions already reacquire via their Allow -> Block transition.
        if let Err(error) = self.requests.close().and_then(|_| self.activate(intent)) {
            self.requests.observation = Observation::MonitoringFailed;
            return Err(error);
        }
        Ok(())
    }

    /// Reports acquisition of the chosen request set; policy may still ignore it.
    pub fn is_active(&self) -> bool {
        self.requests.is_active()
    }

    pub(crate) fn diagnostic_state(&self) -> (Observation, [bool; 2]) {
        (self.requests.observation, self.requests.acquired)
    }
}

impl Drop for SleepBlocker {
    fn drop(&mut self) {
        self.deactivate();
    }
}

// Small private Win32 boundary for fault-injection tests. The production owner
// is monomorphized; there is no worker, shared state, or dynamic dispatch.
trait PowerApi {
    type Handle: Copy;
    fn create(&self) -> Result<Self::Handle, String>;
    fn set(&self, handle: Self::Handle, kind: POWER_REQUEST_TYPE) -> Result<(), String>;
    fn clear(&self, handle: Self::Handle, kind: POWER_REQUEST_TYPE) -> Result<(), String>;
    fn close(&self, handle: Self::Handle) -> Result<(), String>;
}

struct PowerRequests<A: PowerApi> {
    api: A,
    handle: Option<A::Handle>,
    // SYSTEM then DISPLAY, independently of the Win32 enum's numeric values.
    acquired: [bool; 2],
    observation: Observation,
}

impl<A: PowerApi + Default> Default for PowerRequests<A> {
    fn default() -> Self {
        Self {
            api: A::default(),
            handle: None,
            acquired: [false; 2],
            observation: Observation::Inactive,
        }
    }
}

impl<A: PowerApi> PowerRequests<A> {
    fn is_active(&self) -> bool {
        self.observation == Observation::Active
    }

    fn activate(&mut self, intent: PowerIntent) -> Result<(), String> {
        let desired = intent.request_mask();
        if self.is_active() && self.acquired == desired {
            return Ok(());
        }
        if self.is_active() {
            let display_kind = PowerRequestDisplayRequired;
            let Some(handle) = self.handle else {
                return Err("Idle-power requests were active without an owned handle.".to_string());
            };
            if desired[1] && !self.acquired[1] {
                self.api.set(handle, display_kind)?;
            } else if !desired[1] && self.acquired[1] {
                self.api.clear(handle, display_kind)?;
            }
            self.acquired = desired;
            return Ok(());
        }
        self.observation = Observation::Starting;
        let handle = match self.api.create() {
            Ok(handle) => handle,
            Err(error) => {
                self.observation = Observation::ActivationFailed;
                return Err(error);
            }
        };
        self.handle = Some(handle);
        for (index, kind) in REQUEST_TYPES.into_iter().enumerate() {
            if !desired[index] {
                continue;
            }
            if let Err(error) = self.api.set(handle, kind) {
                let result = match self.release() {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => Err(format!("{error}; cleanup: {cleanup_error}")),
                };
                self.observation = Observation::ActivationFailed;
                return result;
            }
            self.acquired[index] = true;
        }
        self.observation = Observation::Active;
        Ok(())
    }

    fn release(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Some(handle) = self.handle {
            for (kind, acquired) in REQUEST_TYPES.into_iter().zip(self.acquired) {
                if acquired {
                    if let Err(error) = self.api.clear(handle, kind) {
                        errors.push(error);
                    }
                }
            }
        }
        // Closing still releases the object if a clear failed. Always attempt
        // both types and CloseHandle, including after a partial acquisition.
        if let Err(error) = self.close() {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn close(&mut self) -> Result<(), String> {
        self.observation = Observation::Inactive;
        self.acquired = [false; 2];
        match self.handle.take() {
            Some(handle) => self.api.close(handle),
            None => Ok(()),
        }
    }
}

impl<A: PowerApi> Drop for PowerRequests<A> {
    fn drop(&mut self) {
        if let Err(error) = self.release() {
            warn!("Idle-power request destruction failed: {error}");
        }
    }
}

#[derive(Default)]
struct WindowsPowerApi;

impl PowerApi for WindowsPowerApi {
    type Handle = HANDLE;

    fn create(&self) -> Result<HANDLE, String> {
        let reason = w!("Wardoff Block mode: manual idle-power protection.");
        let context = REASON_CONTEXT {
            Version: POWER_REQUEST_CONTEXT_VERSION,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                // Microsoft explicitly permits read-only strings here; the API
                // only reads this static, NUL-terminated UTF-16 reason.
                SimpleReasonString: PWSTR(reason.as_ptr().cast_mut()),
            },
        };
        unsafe { PowerCreateRequest(&context) }
            .map_err(|error| format!("PowerCreateRequest failed: {error}"))
    }

    fn set(&self, handle: HANDLE, kind: POWER_REQUEST_TYPE) -> Result<(), String> {
        unsafe { PowerSetRequest(handle, kind) }
            .map_err(|error| format!("PowerSetRequest({kind:?}) failed: {error}"))
    }

    fn clear(&self, handle: HANDLE, kind: POWER_REQUEST_TYPE) -> Result<(), String> {
        unsafe { PowerClearRequest(handle, kind) }
            .map_err(|error| format!("PowerClearRequest({kind:?}) failed: {error}"))
    }

    fn close(&self, handle: HANDLE) -> Result<(), String> {
        unsafe { CloseHandle(handle) }.map_err(|error| format!("CloseHandle failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Call {
        Create,
        Set(i32),
        Clear(i32),
        Close,
    }

    #[derive(Default)]
    struct State {
        calls: Vec<Call>,
        failures: Vec<Call>,
        next_handle: u32,
        // Native enum indices are DISPLAY=0, SYSTEM=1; acquired uses the reverse order.
        live: BTreeMap<u32, [u32; 2]>,
    }

    #[derive(Clone, Default)]
    struct FakeApi(Rc<RefCell<State>>);

    impl FakeApi {
        fn record(&self, call: Call) -> Result<(), String> {
            let mut state = self.0.borrow_mut();
            state.calls.push(call);
            if state.failures.contains(&call) {
                Err(format!("injected {call:?} failure"))
            } else {
                Ok(())
            }
        }
    }

    impl PowerApi for FakeApi {
        type Handle = u32;
        fn create(&self) -> Result<u32, String> {
            self.record(Call::Create)?;
            let mut state = self.0.borrow_mut();
            state.next_handle += 1;
            let handle = state.next_handle;
            state.live.insert(handle, [0; 2]);
            Ok(handle)
        }
        fn set(&self, handle: u32, kind: POWER_REQUEST_TYPE) -> Result<(), String> {
            self.record(Call::Set(kind.0))?;
            self.0.borrow_mut().live.get_mut(&handle).unwrap()[kind.0 as usize] += 1;
            Ok(())
        }
        fn clear(&self, handle: u32, kind: POWER_REQUEST_TYPE) -> Result<(), String> {
            self.record(Call::Clear(kind.0))?;
            let mut state = self.0.borrow_mut();
            let count = &mut state.live.get_mut(&handle).unwrap()[kind.0 as usize];
            assert!(*count > 0, "must not clear an unacquired request");
            *count -= 1;
            Ok(())
        }
        fn close(&self, handle: u32) -> Result<(), String> {
            self.record(Call::Close)?;
            assert!(self.0.borrow_mut().live.remove(&handle).is_some());
            Ok(())
        }
    }

    fn requests() -> (PowerRequests<FakeApi>, FakeApi) {
        let requests = PowerRequests::<FakeApi>::default();
        let api = requests.api.clone();
        (requests, api)
    }

    #[test]
    fn activation_reports_an_acquired_request_before_returning() {
        // Non-disruptive Win32 integration: no sleep, policy writes or elevation.
        let mut blocker = SleepBlocker::default();
        for intent in [PowerIntent::SystemOnly, PowerIntent::SystemAndDisplay] {
            blocker
                .activate(intent)
                .expect("power requests should be available");
            assert!(blocker.is_active());
            assert_eq!(blocker.diagnostic_state().1, intent.request_mask());
            blocker
                .renew_after_resume(intent)
                .expect("renew the owned requests");
            assert!(blocker.is_active());
            assert_eq!(blocker.diagnostic_state().1, intent.request_mask());
            blocker.deactivate();
            assert!(!blocker.is_active());
        }
        blocker
            .activate(PowerIntent::SystemAndDisplay)
            .expect("pair activation should succeed");
        blocker
            .activate(PowerIntent::SystemOnly)
            .expect("active intent flip should be accepted");
        assert_eq!(
            blocker.diagnostic_state().1,
            PowerIntent::SystemOnly.request_mask()
        );
        blocker.deactivate();
    }

    #[test]
    fn repeated_block_allow_cycles_balance_counts_and_close_objects() {
        let (mut requests, api) = requests();
        for _ in 0..100 {
            requests.activate(PowerIntent::default()).unwrap();
            requests.activate(PowerIntent::default()).unwrap();
            assert!(requests.is_active());
            assert_eq!(
                api.0.borrow().live.values().copied().collect::<Vec<_>>(),
                vec![[1, 1]]
            );
            requests.release().unwrap();
            requests.release().unwrap();
            assert!(!requests.is_active());
            assert!(api.0.borrow().live.is_empty());
        }
    }

    #[test]
    fn failed_creation_does_not_claim_protection_or_close_an_invalid_handle() {
        let (mut requests, api) = requests();
        api.0.borrow_mut().failures = vec![Call::Create];
        assert!(requests.activate(PowerIntent::default()).is_err());
        assert!(!requests.is_active());
        drop(requests);
        assert_eq!(api.0.borrow().calls, vec![Call::Create]);
    }

    #[test]
    fn either_set_failure_rolls_back_only_acquired_requests_and_can_retry() {
        for kind in REQUEST_TYPES {
            let (mut requests, api) = requests();
            api.0.borrow_mut().failures = vec![Call::Set(kind.0)];
            assert!(requests.activate(PowerIntent::default()).is_err());
            assert!(!requests.is_active());
            assert!(api.0.borrow().live.is_empty());
            assert!(!api.0.borrow().calls.contains(&Call::Clear(kind.0)));
            api.0.borrow_mut().failures.clear();
            requests.activate(PowerIntent::default()).unwrap();
            assert!(requests.is_active());
            drop(requests);
            assert!(api.0.borrow().live.is_empty());
        }
    }

    #[test]
    fn cleanup_continues_after_clear_errors_and_reports_each_error() {
        let (mut requests, api) = requests();
        requests.activate(PowerIntent::default()).unwrap();
        api.0.borrow_mut().failures = vec![Call::Clear(1), Call::Clear(0)];
        let error = requests.release().unwrap_err();
        assert!(error.contains("Clear(1)"));
        assert!(error.contains("Clear(0)"));
        assert!(api.0.borrow().live.is_empty());
        assert!(!requests.is_active());
    }

    #[test]
    fn partial_acquisition_preserves_original_and_cleanup_errors() {
        let (mut requests, api) = requests();
        api.0.borrow_mut().failures = vec![Call::Set(0), Call::Clear(1)];
        let error = requests.activate(PowerIntent::default()).unwrap_err();
        assert!(error.contains("Set(0)"));
        assert!(error.contains("Clear(1)"));
        assert!(api.0.borrow().live.is_empty());
    }

    #[test]
    fn drop_releases_both_requests_without_explicit_deactivation() {
        let (mut requests, api) = requests();
        requests.activate(PowerIntent::default()).unwrap();
        drop(requests);
        assert!(api.0.borrow().live.is_empty());
        assert!(api
            .0
            .borrow()
            .calls
            .ends_with(&[Call::Clear(1), Call::Clear(0), Call::Close]));
    }

    #[test]
    fn resume_replacement_handles_ended_or_surviving_counts_without_accumulation() {
        for counts in [[0, 0], [1, 1]] {
            let (mut requests, api) = requests();
            requests.activate(PowerIntent::default()).unwrap();
            *api.0.borrow_mut().live.values_mut().next().unwrap() = counts;
            // This validates ownership, not real Windows suspend behavior.
            for _ in 0..2 {
                requests.close().unwrap();
                requests.activate(PowerIntent::default()).unwrap();
                assert_eq!(
                    api.0.borrow().live.values().copied().collect::<Vec<_>>(),
                    vec![[1, 1]]
                );
            }
        }
    }

    #[test]
    fn system_only_acquires_no_display_request() {
        let (mut requests, api) = requests();
        requests.activate(PowerIntent::SystemOnly).unwrap();

        assert_eq!(requests.acquired, [true, false]);
        assert_eq!(
            api.0.borrow().live.values().copied().collect::<Vec<_>>(),
            vec![[0, 1]]
        );
        assert!(requests.is_active());
    }

    #[test]
    fn active_intent_flip_keeps_one_system_request_without_duplication() {
        let (mut requests, api) = requests();
        requests.activate(PowerIntent::SystemAndDisplay).unwrap();
        let initial_calls = api.0.borrow().calls.len();
        requests.activate(PowerIntent::SystemOnly).unwrap();
        assert_eq!(requests.acquired, [true, false]);
        requests.activate(PowerIntent::SystemAndDisplay).unwrap();

        assert_eq!(requests.acquired, [true, true]);
        assert_eq!(
            api.0.borrow().live.values().copied().collect::<Vec<_>>(),
            vec![[1, 1]]
        );
        let calls = &api.0.borrow().calls;
        assert_eq!(
            calls.iter().filter(|call| **call == Call::Set(1)).count(),
            1
        );
        assert!(!calls[initial_calls..].contains(&Call::Set(1)));
        assert!(!calls[initial_calls..].contains(&Call::Clear(1)));
        assert!(!calls[initial_calls..].contains(&Call::Close));
        assert!(!calls[initial_calls..].contains(&Call::Create));
    }

    #[test]
    fn repeated_same_intent_is_a_noop() {
        let (mut requests, api) = requests();
        requests.activate(PowerIntent::SystemOnly).unwrap();
        let calls = api.0.borrow().calls.clone();
        requests.activate(PowerIntent::SystemOnly).unwrap();
        assert_eq!(api.0.borrow().calls, calls);
    }

    #[test]
    fn failed_display_add_preserves_system_only_and_active_state() {
        let (mut requests, api) = requests();
        requests.activate(PowerIntent::SystemOnly).unwrap();
        api.0.borrow_mut().failures = vec![Call::Set(0)];
        assert!(requests.activate(PowerIntent::SystemAndDisplay).is_err());
        assert_eq!(requests.acquired, [true, false]);
        assert_eq!(
            api.0.borrow().live.values().copied().collect::<Vec<_>>(),
            vec![[0, 1]]
        );
        assert!(requests.is_active());
    }

    #[test]
    fn failed_display_clear_preserves_pair_and_active_state() {
        let (mut requests, api) = requests();
        requests.activate(PowerIntent::SystemAndDisplay).unwrap();
        api.0.borrow_mut().failures = vec![Call::Clear(0)];
        assert!(requests.activate(PowerIntent::SystemOnly).is_err());
        assert_eq!(requests.acquired, [true, true]);
        assert_eq!(
            api.0.borrow().live.values().copied().collect::<Vec<_>>(),
            vec![[1, 1]]
        );
        assert!(requests.is_active());
    }

    #[test]
    fn resume_and_cleanup_balance_each_intent() {
        for intent in [PowerIntent::SystemOnly, PowerIntent::SystemAndDisplay] {
            let (mut requests, api) = requests();
            requests.activate(intent).unwrap();
            requests.close().unwrap();
            requests.activate(intent).unwrap();
            assert_eq!(requests.acquired, intent.request_mask());
            assert_eq!(
                api.0.borrow().live.values().copied().collect::<Vec<_>>(),
                vec![[u32::from(intent == PowerIntent::SystemAndDisplay), 1]]
            );
            requests.release().unwrap();
            assert!(api.0.borrow().live.is_empty());
        }
    }
}
