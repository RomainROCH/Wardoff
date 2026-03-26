use windows::core::{Error as WindowsError, HRESULT};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_NOT_ALL_ASSIGNED, ERROR_NO_SHUTDOWN_IN_PROGRESS, E_ACCESSDENIED,
};
use windows::Win32::System::Shutdown::AbortSystemShutdownW;

/// Describes whether this process can currently use `AbortSystemShutdownW(None)`.
#[derive(Debug)]
pub enum ShutdownAbortCapability {
    /// The process can enable the required privilege and attempt shutdown aborts.
    Available,
    /// Windows denied the required privilege or capability.
    AccessDenied { details: String },
    /// Windows failed the capability check for another reason.
    Failed { details: String },
}

/// Describes the result of calling `AbortSystemShutdownW(None)`.
#[derive(Debug)]
pub enum ShutdownAbortOutcome {
    /// Windows reported that a pending shutdown was aborted.
    Aborted,
    /// No abortable shutdown was pending at the time of the call.
    NoShutdownPending,
    /// The current process lacks the privilege Windows requires for the abort call.
    AccessDenied { details: String },
    /// Windows rejected the abort attempt for another reason.
    Failed { details: String },
}

/// Verifies whether this process can attempt `AbortSystemShutdownW(None)` before starting a layer.
pub fn preflight_shutdown_abort_capability() -> ShutdownAbortCapability {
    match super::enable_shutdown_privilege() {
        Ok(()) => ShutdownAbortCapability::Available,
        Err(error) if is_access_denied_error(&error) => ShutdownAbortCapability::AccessDenied {
            details: error.to_string(),
        },
        Err(error) => ShutdownAbortCapability::Failed {
            details: error.to_string(),
        },
    }
}

/// Attempts to cancel a pending shutdown on the local machine.
pub fn abort_pending_shutdown() -> ShutdownAbortOutcome {
    match preflight_shutdown_abort_capability() {
        ShutdownAbortCapability::Available => {}
        ShutdownAbortCapability::AccessDenied { details } => {
            return ShutdownAbortOutcome::AccessDenied { details };
        }
        ShutdownAbortCapability::Failed { details } => {
            return ShutdownAbortOutcome::Failed { details };
        }
    }

    match unsafe { AbortSystemShutdownW(None) } {
        Ok(()) => ShutdownAbortOutcome::Aborted,
        Err(error) if is_no_shutdown_in_progress_error(&error) => {
            ShutdownAbortOutcome::NoShutdownPending
        }
        Err(error) if is_access_denied_error(&error) => ShutdownAbortOutcome::AccessDenied {
            details: error.to_string(),
        },
        Err(error) => ShutdownAbortOutcome::Failed {
            details: error.to_string(),
        },
    }
}

fn is_access_denied_error(error: &WindowsError) -> bool {
    let code = error.code();
    code == E_ACCESSDENIED
        || code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
        || code == HRESULT::from_win32(ERROR_NOT_ALL_ASSIGNED.0)
}

fn is_no_shutdown_in_progress_error(error: &WindowsError) -> bool {
    error.code() == HRESULT::from_win32(ERROR_NO_SHUTDOWN_IN_PROGRESS.0)
}
