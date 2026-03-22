use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

/// Owns the global mutex used to enforce a single primary Wardoff runtime.
pub(crate) struct InstanceGuard(HANDLE);

/// Describes whether the current process became the primary Wardoff runtime.
pub(crate) enum InstanceClaim {
    /// The current process successfully claimed the named mutex.
    Primary(InstanceGuard),
    /// Another Wardoff process already owns the named mutex.
    Secondary,
}

/// Attempts to claim the named Wardoff mutex for the current process.
pub(crate) fn claim_primary_instance() -> Result<InstanceClaim, String> {
    let handle = unsafe { CreateMutexW(None, false, w!("Global\\WardoffInstance")) }
        .map_err(|error| format!("Wardoff could not create its single-instance mutex: {error}"))?;

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        Ok(InstanceClaim::Secondary)
    } else {
        Ok(InstanceClaim::Primary(InstanceGuard(handle)))
    }
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
