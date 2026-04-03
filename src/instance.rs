use std::thread;
use std::time::Duration;
use windows::core::{w, Error as WindowsError, HRESULT};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, E_ACCESSDENIED, HANDLE,
};
use windows::Win32::System::Threading::{CreateMutexExW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

/// Owns the global mutex used to enforce a single primary Wardoff runtime.
pub(crate) struct InstanceGuard(HANDLE);

/// Describes whether the current process became the primary Wardoff runtime.
pub(crate) enum InstanceClaim {
    /// The current process successfully claimed the named mutex.
    Primary(InstanceGuard),
    /// Another Wardoff process already owns the named mutex.
    Secondary,
}

#[derive(Debug, Eq, PartialEq)]
enum ClaimDisposition {
    Primary,
    Secondary,
}

/// Attempts to claim the named Wardoff mutex for the current process.
pub(crate) fn claim_primary_instance() -> Result<InstanceClaim, String> {
    let handle = match unsafe {
        CreateMutexExW(
            None,
            w!("Global\\WardoffInstance"),
            0,
            SYNCHRONIZATION_SYNCHRONIZE.0,
        )
    } {
        Ok(handle) => handle,
        Err(create_error) if is_access_denied_error(&create_error) => {
            return match classify_access_denied_probe(
                &create_error,
                match unsafe {
                    OpenMutexW(
                        SYNCHRONIZATION_SYNCHRONIZE,
                        false,
                        w!("Global\\WardoffInstance"),
                    )
                } {
                    Ok(handle) => {
                        unsafe {
                            let _ = CloseHandle(handle);
                        }
                        Ok(())
                    }
                    Err(open_error) => Err(open_error),
                },
            )? {
                ClaimDisposition::Primary => {
                    unreachable!("access-denied probe cannot produce primary")
                }
                ClaimDisposition::Secondary => Ok(InstanceClaim::Secondary),
            };
        }
        Err(error) => {
            return Err(format!(
                "Wardoff could not create or open its single-instance mutex (Global\\WardoffInstance): {error}"
            ));
        }
    };

    if classify_create_claim(unsafe { GetLastError() } == ERROR_ALREADY_EXISTS)
        == ClaimDisposition::Secondary
    {
        unsafe {
            let _ = CloseHandle(handle);
        }
        Ok(InstanceClaim::Secondary)
    } else {
        Ok(InstanceClaim::Primary(InstanceGuard(handle)))
    }
}

fn classify_create_claim(already_exists: bool) -> ClaimDisposition {
    if already_exists {
        ClaimDisposition::Secondary
    } else {
        ClaimDisposition::Primary
    }
}

fn is_access_denied_error(error: &WindowsError) -> bool {
    let code = error.code();
    code == E_ACCESSDENIED || code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
}

fn classify_access_denied_probe(
    create_error: &WindowsError,
    open_result: Result<(), WindowsError>,
) -> Result<ClaimDisposition, String> {
    match open_result {
        Ok(()) => Ok(ClaimDisposition::Secondary),
        Err(open_error) => Err(format!(
            "Wardoff could not create or open its single-instance mutex (Global\\WardoffInstance). The environment or an existing owner may be restricting access. CreateMutexExW: {create_error}; OpenMutexW probe: {open_error}"
        )),
    }
}

/// Retries the primary-instance claim to bridge short handoff windows during internal relaunch.
pub(crate) fn claim_primary_instance_with_retry(
    attempts: usize,
    delay: Duration,
) -> Result<InstanceClaim, String> {
    let attempts = attempts.max(1);

    for attempt in 0..attempts {
        let claim = claim_primary_instance()?;
        if matches!(claim, InstanceClaim::Primary(_)) || attempt + 1 == attempts {
            return Ok(claim);
        }

        thread::sleep(delay);
    }

    unreachable!("the retry loop always returns on its final attempt")
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_access_denied_probe, classify_create_claim, is_access_denied_error,
        ClaimDisposition,
    };
    use windows::core::{Error as WindowsError, HRESULT};
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;

    fn win32_error(code: u32) -> WindowsError {
        WindowsError::from_hresult(HRESULT::from_win32(code))
    }

    #[test]
    fn newly_created_mutex_is_primary() {
        assert_eq!(classify_create_claim(false), ClaimDisposition::Primary);
    }

    #[test]
    fn existing_mutex_is_secondary() {
        assert_eq!(classify_create_claim(true), ClaimDisposition::Secondary);
    }

    #[test]
    fn access_denied_with_successful_open_is_secondary() {
        assert_eq!(
            classify_access_denied_probe(&win32_error(ERROR_ACCESS_DENIED.0), Ok(())),
            Ok(ClaimDisposition::Secondary)
        );
    }

    #[test]
    fn access_denied_with_failed_open_is_actionable_error() {
        let error = classify_access_denied_probe(
            &win32_error(ERROR_ACCESS_DENIED.0),
            Err(win32_error(ERROR_ACCESS_DENIED.0)),
        )
        .expect_err("expected actionable error");

        assert!(error.contains("may be restricting access"));
        assert!(error.contains("CreateMutexExW"));
        assert!(error.contains("OpenMutexW probe"));
    }

    #[test]
    fn access_denied_detection_uses_error_object() {
        assert!(is_access_denied_error(&win32_error(ERROR_ACCESS_DENIED.0)));
    }
}
