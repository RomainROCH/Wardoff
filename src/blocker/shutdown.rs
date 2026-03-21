use windows::core::Result as WindowsResult;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::WM_QUERYENDSESSION;

/// Owns the Layer 1 message-only window used for interactive shutdown blocking.
pub struct ShutdownBlocker;

/// Creates the message-only window and associates the shutdown block reason.
pub fn create_shutdown_blocker_window() -> WindowsResult<HWND> {
    todo!("Create the message-only window, register the shutdown block reason, and raise shutdown priority")
}

/// Handles `WM_QUERYENDSESSION` while Wardoff is in blocking mode.
pub fn handle_query_end_session(_wparam: WPARAM, _lparam: LPARAM) -> LRESULT {
    todo!("Inspect WM_QUERYENDSESSION and refuse shutdown while blocking is enabled")
}

/// Returns the Windows message identifier used for interactive shutdown negotiation.
pub fn query_end_session_message() -> u32 {
    todo!("Expose the WM_QUERYENDSESSION constant for the shutdown message loop scaffolding")
}
