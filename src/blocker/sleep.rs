use crate::logger::{self, EventSource};
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
    /// Acquires both requests synchronously, rolling back any partial acquisition.
    pub fn activate(&mut self) -> Result<(), String> {
        if self.is_active() {
            return Ok(());
        }
        if let Err(error) = self.requests.activate() {
            warn!("Could not acquire idle-power requests: {error}");
            logger::log_event("sleep_blocked", EventSource::Sleep, &error, false);
            return Err(error);
        }
        let message = "Wardoff acquired system/display Power Requests for idle-sleep and automatic display-timeout prevention. Windows policy and explicit Sleep/Hibernate can override this protection.";
        info!("{message}");
        // Keep the existing event name for log consumers; success means request
        // acquisition, not that an actual sleep transition was observed or vetoed.
        logger::log_event("sleep_blocked", EventSource::Sleep, message, true);
        Ok(())
    }

    /// Releases the requests and closes their private process-owned handle.
    pub fn deactivate(&mut self) {
        if self.requests.handle.is_none() {
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
    pub(crate) fn renew_after_resume(&mut self) -> Result<(), String> {
        // PowerSetRequest documents termination at user-initiated sleep entry:
        // https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-powersetrequest
        // The handle need not be invalid. Close the old object instead of
        // incrementing possibly surviving counts or clearing already-ended ones.
        // Tray power actions already reacquire via their Allow -> Block transition.
        self.requests.close()?;
        self.activate()
    }

    /// Reports acquisition of the pair; Windows policy may still ignore it.
    pub fn is_active(&self) -> bool {
        self.requests.is_active()
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
    acquired: [bool; 2],
}

impl<A: PowerApi + Default> Default for PowerRequests<A> {
    fn default() -> Self {
        Self {
            api: A::default(),
            handle: None,
            acquired: [false; 2],
        }
    }
}

impl<A: PowerApi> PowerRequests<A> {
    fn is_active(&self) -> bool {
        self.handle.is_some() && self.acquired.iter().all(|acquired| *acquired)
    }

    fn activate(&mut self) -> Result<(), String> {
        if self.is_active() {
            return Ok(());
        }
        let handle = self.api.create()?;
        self.handle = Some(handle);
        for (index, kind) in REQUEST_TYPES.into_iter().enumerate() {
            if let Err(error) = self.api.set(handle, kind) {
                return match self.release() {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => Err(format!("{error}; cleanup: {cleanup_error}")),
                };
            }
            self.acquired[index] = true;
        }
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
        let reason = w!("Wardoff Block mode: prevent idle sleep and automatic display timeout.");
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
        for _ in 0..10 {
            blocker
                .activate()
                .expect("power requests should be available");
            assert!(blocker.is_active());
            blocker
                .renew_after_resume()
                .expect("renew the owned requests");
            assert!(blocker.is_active());
            blocker.deactivate();
            assert!(!blocker.is_active());
        }
    }

    #[test]
    fn repeated_block_allow_cycles_balance_counts_and_close_objects() {
        let (mut requests, api) = requests();
        for _ in 0..100 {
            requests.activate().unwrap();
            requests.activate().unwrap();
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
        assert!(requests.activate().is_err());
        assert!(!requests.is_active());
        drop(requests);
        assert_eq!(api.0.borrow().calls, vec![Call::Create]);
    }

    #[test]
    fn either_set_failure_rolls_back_only_acquired_requests_and_can_retry() {
        for kind in REQUEST_TYPES {
            let (mut requests, api) = requests();
            api.0.borrow_mut().failures = vec![Call::Set(kind.0)];
            assert!(requests.activate().is_err());
            assert!(!requests.is_active());
            assert!(api.0.borrow().live.is_empty());
            assert!(!api.0.borrow().calls.contains(&Call::Clear(kind.0)));
            api.0.borrow_mut().failures.clear();
            requests.activate().unwrap();
            assert!(requests.is_active());
            drop(requests);
            assert!(api.0.borrow().live.is_empty());
        }
    }

    #[test]
    fn cleanup_continues_after_clear_errors_and_reports_each_error() {
        let (mut requests, api) = requests();
        requests.activate().unwrap();
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
        let error = requests.activate().unwrap_err();
        assert!(error.contains("Set(0)"));
        assert!(error.contains("Clear(1)"));
        assert!(api.0.borrow().live.is_empty());
    }

    #[test]
    fn drop_releases_both_requests_without_explicit_deactivation() {
        let (mut requests, api) = requests();
        requests.activate().unwrap();
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
            requests.activate().unwrap();
            *api.0.borrow_mut().live.values_mut().next().unwrap() = counts;
            // This validates ownership, not real Windows suspend behavior.
            for _ in 0..2 {
                requests.close().unwrap();
                requests.activate().unwrap();
                assert_eq!(
                    api.0.borrow().live.values().copied().collect::<Vec<_>>(),
                    vec![[1, 1]]
                );
            }
        }
    }
}
