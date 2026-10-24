//! Exercises the production singleton claim and guard on isolated Windows mutexes.
//! No protection runtime, production session mutex, power request, or autostart is used.
#![cfg(windows)]

use std::time::{Duration, Instant};
use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE};
use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

mod session_scope {
    use std::cell::RefCell;
    use windows::Win32::System::Com::CoCreateGuid;

    struct ProbeAction {
        at_probe: usize,
        callback: Box<dyn FnOnce()>,
    }

    struct Context {
        name: String,
        probes: usize,
        action: Option<ProbeAction>,
    }

    thread_local! {
        static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
    }

    pub(crate) struct MutexScope {
        name: String,
        previous: Option<Context>,
    }

    impl MutexScope {
        pub(crate) fn new(label: &str) -> Self {
            let unique = unsafe { CoCreateGuid() }.expect("create isolated mutex GUID");
            let name = format!(
                r"Local\WardoffInstanceLifecycle-{label}-{}-{unique:?}",
                std::process::id()
            );
            let previous = CONTEXT.with(|context| {
                context.replace(Some(Context {
                    name: name.clone(),
                    probes: 0,
                    action: None,
                }))
            });
            Self { name, previous }
        }

        pub(crate) fn name(&self) -> &str {
            &self.name
        }

        pub(crate) fn probes(&self) -> usize {
            CONTEXT.with(|context| {
                let context = context.borrow();
                let context = context.as_ref().expect("active isolated mutex scope");
                assert_eq!(context.name, self.name);
                context.probes
            })
        }

        pub(crate) fn reset_probes(&self) {
            CONTEXT.with(|context| {
                let mut context = context.borrow_mut();
                let context = context.as_mut().expect("active isolated mutex scope");
                assert_eq!(context.name, self.name);
                context.probes = 0;
            });
        }

        pub(crate) fn release_owner_on_probe(
            &self,
            at_probe: usize,
            owner: crate::instance::InstanceGuard,
        ) {
            assert!(at_probe > 0);
            CONTEXT.with(|context| {
                let mut context = context.borrow_mut();
                let context = context.as_mut().expect("active isolated mutex scope");
                assert_eq!(context.name, self.name);
                assert!(
                    context.action.is_none(),
                    "only one release action is needed"
                );
                context.probes = 0;
                context.action = Some(ProbeAction {
                    at_probe,
                    callback: Box::new(move || drop(owner)),
                });
            });
        }
    }

    impl Drop for MutexScope {
        fn drop(&mut self) {
            let current = CONTEXT.with(|context| context.replace(self.previous.take()));
            // Dropping an unused callback also drops its captured owner during unwinding.
            // Both happen after the TLS borrow ends and the previous scope is restored.
            drop(current);
        }
    }

    pub(crate) fn current_mutex_name() -> Result<String, String> {
        let (name, action) = CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let context = context.as_mut().ok_or_else(|| {
                "The isolated mutex test requires an explicit test scope.".to_string()
            })?;
            context.probes += 1;
            let action = if context
                .action
                .as_ref()
                .is_some_and(|action| action.at_probe == context.probes)
            {
                context.action.take()
            } else {
                None
            };
            Ok::<_, String>((context.name.clone(), action))
        })?;
        // The second resolver entry follows a completed first Secondary claim and
        // its retry pause. Close the real owner before the second CreateMutexExW.
        if let Some(action) = action {
            (action.callback)();
        }
        Ok(name)
    }
}

#[path = "../src/instance.rs"]
mod instance;

fn primary(claim: Result<instance::InstanceClaim, String>) -> instance::InstanceGuard {
    match claim.expect("claim isolated mutex") {
        instance::InstanceClaim::Primary(guard) => guard,
        instance::InstanceClaim::Secondary => panic!("expected isolated mutex to be Primary"),
    }
}

fn assert_secondary(claim: Result<instance::InstanceClaim, String>) {
    assert!(
        matches!(
            claim.expect("claim existing isolated mutex"),
            instance::InstanceClaim::Secondary
        ),
        "expected isolated mutex to remain Secondary while its guard is held"
    );
}

struct ProbeHandle(HANDLE);

impl Drop for ProbeHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn assert_mutex_removed(scope: &session_scope::MutexScope) {
    let name: Vec<u16> = scope
        .name()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) } {
        Ok(handle) => {
            let _unexpected_probe = ProbeHandle(handle);
            panic!("isolated mutex survived the final guard drop");
        }
        Err(error) => assert_eq!(
            error.code(),
            HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0),
            "unexpected error while checking isolated mutex cleanup: {error}"
        ),
    }
}

#[test]
fn native_claim_lifecycle_releases_primary_and_temporary_secondary_handles() {
    let scope = session_scope::MutexScope::new("lifecycle");
    let owner = primary(instance::claim_primary_instance());
    for _ in 0..3 {
        assert_secondary(instance::claim_primary_instance());
    }
    assert_eq!(scope.probes(), 4);

    drop(owner);
    let replacement = primary(instance::claim_primary_instance());
    assert_eq!(scope.probes(), 5);
    drop(replacement);
    assert_mutex_removed(&scope);
}

#[test]
fn doctor_observation_never_reserves_or_retains_a_primary_slot() {
    let scope = session_scope::MutexScope::new("doctor-observation");
    for _ in 0..20 {
        assert!(!instance::primary_instance_exists().unwrap());
        assert_mutex_removed(&scope);
    }
    let owner = primary(instance::claim_primary_instance());
    assert!(instance::primary_instance_exists().unwrap());
    drop(owner);
    assert!(!instance::primary_instance_exists().unwrap());
    let replacement = primary(instance::claim_primary_instance());
    drop(replacement);
    assert_mutex_removed(&scope);
}

#[test]
fn native_retry_exhaustion_returns_secondary_and_does_not_retain_handles() {
    let scope = session_scope::MutexScope::new("exhaustion");
    let owner = primary(instance::claim_primary_instance());
    scope.reset_probes();

    let started = Instant::now();
    let claim = instance::claim_primary_instance_with_retry(3, Duration::from_millis(20));
    let elapsed = started.elapsed();
    assert_secondary(claim);
    assert_eq!(scope.probes(), 3);
    eprintln!("held mutex: 3 probes, nominal 40 ms retry pauses, observed {elapsed:?}");

    drop(owner);
    let replacement = primary(instance::claim_primary_instance());
    drop(replacement);
    assert_mutex_removed(&scope);
}

#[test]
fn native_retry_reclaims_after_owner_is_released_at_second_probe() {
    let scope = session_scope::MutexScope::new("handoff");
    let owner = primary(instance::claim_primary_instance());
    scope.release_owner_on_probe(2, owner);

    let started = Instant::now();
    let replacement = primary(instance::claim_primary_instance_with_retry(
        3,
        Duration::from_millis(20),
    ));
    let elapsed = started.elapsed();
    assert_eq!(scope.probes(), 2);
    eprintln!("released mutex: 2 probes, nominal 20 ms retry pause, observed {elapsed:?}");
    assert_secondary(instance::claim_primary_instance());

    drop(replacement);
    let reclaimed = primary(instance::claim_primary_instance());
    drop(reclaimed);
    assert_mutex_removed(&scope);
}

#[test]
fn native_zero_attempts_still_claims_once_for_free_and_held_mutexes() {
    let scope = session_scope::MutexScope::new("zero-attempts");
    let started = Instant::now();
    let owner = primary(instance::claim_primary_instance_with_retry(
        0,
        Duration::from_secs(1),
    ));
    let free_elapsed = started.elapsed();
    assert_eq!(scope.probes(), 1);

    scope.reset_probes();
    let started = Instant::now();
    let claim = instance::claim_primary_instance_with_retry(0, Duration::from_secs(1));
    let held_elapsed = started.elapsed();
    assert_secondary(claim);
    assert_eq!(scope.probes(), 1);
    eprintln!("zero attempts: one probe each, free {free_elapsed:?}, held {held_elapsed:?}");

    drop(owner);
    let replacement = primary(instance::claim_primary_instance());
    drop(replacement);
    assert_mutex_removed(&scope);
}
