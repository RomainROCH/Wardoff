//! Layer 1 covers the standard, official Windows shutdown negotiation path:
//! Wardoff raises its shutdown priority, registers a shutdown block reason, and
//! returns `FALSE` from `WM_QUERYENDSESSION` while blocking is active so Windows
//! can present the normal "this app is preventing shutdown" flow. It does not
//! cover forced local shutdowns such as `shutdown /t 0 /f`, ETW/IFEO-based
//! interception, Windows Update task handling, remote abort polling, timers,
//! profiles, tray UI, or any other later-layer features.
//!
//! Windows does not broadcast `WM_QUERYENDSESSION` to `HWND_MESSAGE` windows, so
//! this MVP keeps both the requested message-only window and a hidden top-level
//! companion window. The hidden top-level window owns the actual shutdown block
//! reason and receives the broadcast shutdown query.

use crate::logger::{self, EventSource};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;

use log::warn;
use windows::core::{w, Error as WindowsError, Result as WindowsResult, HSTRING};
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification, HPOWERNOTIFY,
};
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::Threading::SetProcessShutdownParameters;
use windows::Win32::System::WindowsProgramming::SHUTDOWN_NORETRY;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, IsWindow, RegisterClassW,
    DEVICE_NOTIFY_WINDOW_HANDLE, ENDSESSION_CLOSEAPP, ENDSESSION_CRITICAL, ENDSESSION_LOGOFF,
    HWND_MESSAGE, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, SM_SHUTTINGDOWN,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_ENDSESSION, WM_POWERBROADCAST,
    WM_QUERYENDSESSION, WNDCLASSW, WS_OVERLAPPED,
};

static BLOCKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static SESSION_DIAGNOSTICS: Mutex<SessionDiagnostics> = Mutex::new(SessionDiagnostics::new());
static WINDOW_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();
static END_SESSION_CLEANUP_CALLBACK: Mutex<Option<fn()>> = Mutex::new(None);
static END_SESSION_CLEANUP_STARTED: AtomicBool = AtomicBool::new(false);
static POWER_BROADCAST_CALLBACK: Mutex<Option<fn(PowerBroadcastEvent)>> = Mutex::new(None);

const SHUTDOWN_PRIORITY: u32 = 0x3FF;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SessionQuery {
    window: isize,
    wparam: usize,
    lparam: isize,
}

// Diagnostic correlation only: no transition here changes the protection decision.
// Keep the two Layer 1 windows separate and consume a refusal at most once.
struct SessionDiagnostics {
    refusals: Vec<SessionQuery>,
}

impl SessionDiagnostics {
    const fn new() -> Self {
        Self {
            refusals: Vec::new(),
        }
    }

    fn forget_window(&mut self, window: isize) -> Option<SessionQuery> {
        let index = self
            .refusals
            .iter()
            .position(|query| query.window == window)?;
        Some(self.refusals.swap_remove(index))
    }

    fn query(&mut self, query: SessionQuery, refused: bool) {
        let _ = self.forget_window(query.window);
        if refused {
            self.refusals.push(query);
        }
    }

    fn end(&mut self, window: isize, ending: bool) -> SessionOutcome {
        SessionOutcome {
            ending,
            refused_query: self.forget_window(window),
        }
    }
}

#[derive(Debug)]
struct SessionOutcome {
    ending: bool,
    refused_query: Option<SessionQuery>,
}

impl SessionOutcome {
    fn confirmed_cancellation_after_refusal(&self) -> bool {
        !self.ending && self.refused_query.is_some()
    }

    fn event(&self) -> &'static str {
        if self.ending {
            "session_end_proceeding"
        } else {
            "session_end_cancelled"
        }
    }

    fn action(&self, wparam: usize, lparam: isize) -> String {
        let query_details = self
            .refused_query
            .map(|query| session_message_details(query.wparam, query.lparam));
        format!(
            "WM_ENDSESSION: {}; prior refused query={query_details:?}. Windows reports the session {}; the initiator and cancellation cause are unknown.",
            session_message_details(wparam, lparam),
            if self.ending { "is ending" } else { "is not ending" },
        )
    }
}

fn session_message_details(wparam: usize, lparam: isize) -> String {
    let flags = lparam as u32;
    format!(
        "wParam={wparam}, lParam={:#x}, logoff={}, critical={}, close_app={}",
        lparam as usize,
        flags & ENDSESSION_LOGOFF != 0,
        flags & ENDSESSION_CRITICAL != 0,
        flags & ENDSESSION_CLOSEAPP != 0,
    )
}

/// Represents the suspend/resume notifications Wardoff listens for on its hidden window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PowerBroadcastEvent {
    /// Windows is entering a suspend state.
    Suspend,
    /// Windows resumed from suspend or hibernate.
    Resume,
}

/// Owns the Layer 1 windows used for interactive shutdown blocking.
pub struct ShutdownBlocker {
    message_window: HWND,
    session_window: HWND,
    suspend_resume_notification: Option<HPOWERNOTIFY>,
    reason: HSTRING,
    reason_registered: bool,
}

impl ShutdownBlocker {
    /// Creates the Layer 1 blocker, including the requested message-only window
    /// and the hidden top-level window that actually receives shutdown queries.
    /// The blocker starts in Allow mode until `activate` is called.
    pub fn new(reason: &str) -> WindowsResult<Self> {
        let hinstance = current_instance()?;
        register_window_class(hinstance)?;

        let message_window = create_message_window(hinstance)?;
        let session_window = match create_session_window(hinstance) {
            Ok(window) => window,
            Err(error) => {
                destroy_window(message_window);
                return Err(error);
            }
        };

        if let Err(error) =
            unsafe { SetProcessShutdownParameters(SHUTDOWN_PRIORITY, SHUTDOWN_NORETRY) }
        {
            destroy_window(session_window);
            destroy_window(message_window);
            return Err(error);
        }

        // Since Windows 8, PBT_APMSUSPEND/PBT_APMRESUMEAUTOMATIC/PBT_APMRESUMESUSPEND are no longer
        // broadcast to top-level windows automatically; the recipient must register explicitly,
        // otherwise WM_POWERBROADCAST never reaches `shutdown_window_proc` for those events.
        let suspend_resume_notification = match register_suspend_resume_notification(session_window)
        {
            Ok(handle) => Some(handle),
            Err(error) => {
                destroy_window(session_window);
                destroy_window(message_window);
                return Err(error);
            }
        };

        Ok(Self {
            message_window,
            session_window,
            suspend_resume_notification,
            reason: HSTRING::from(reason),
            reason_registered: false,
        })
    }

    /// Enables Layer 1 shutdown blocking without destroying the UI thread windows.
    pub fn activate(&mut self) -> WindowsResult<()> {
        if self.reason_registered {
            return Ok(());
        }

        unsafe { ShutdownBlockReasonCreate(self.session_window, &self.reason)? };
        self.reason_registered = true;
        BLOCKER_ACTIVE.store(true, Ordering::Release);
        logger::log_event(
            "shutdown_layer_enabled",
            EventSource::Shutdown,
            "Layer 1 registered its shutdown block reason and will reject WM_QUERYENDSESSION.",
            true,
        );
        Ok(())
    }

    /// Disables Layer 1 shutdown blocking while keeping the UI thread windows alive.
    pub fn deactivate(&mut self) -> WindowsResult<()> {
        if !self.reason_registered {
            BLOCKER_ACTIVE.store(false, Ordering::Release);
            return Ok(());
        }

        unsafe { ShutdownBlockReasonDestroy(self.session_window)? };
        self.reason_registered = false;
        BLOCKER_ACTIVE.store(false, Ordering::Release);
        logger::log_event(
            "shutdown_layer_disabled",
            EventSource::Shutdown,
            "Layer 1 removed its shutdown block reason and returned to Allow mode.",
            true,
        );
        Ok(())
    }

    /// Returns whether Layer 1 is currently blocking shutdown.
    pub fn is_active(&self) -> bool {
        self.reason_registered
    }
}

impl Drop for ShutdownBlocker {
    fn drop(&mut self) {
        let _ = self.deactivate();
        if let Some(handle) = self.suspend_resume_notification.take() {
            unsafe {
                let _ = UnregisterSuspendResumeNotification(handle);
            }
        }
        destroy_window(self.session_window);
        destroy_window(self.message_window);
        BLOCKER_ACTIVE.store(false, Ordering::Release);
    }
}

/// Handles `WM_QUERYENDSESSION` for shutdown and sign-out while Wardoff is in blocking mode.
pub fn handle_query_end_session(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let refused = BLOCKER_ACTIVE.load(Ordering::Acquire);
    if let Ok(mut diagnostics) = SESSION_DIAGNOSTICS.lock() {
        diagnostics.query(
            SessionQuery {
                window: hwnd.0 as isize,
                wparam: wparam.0,
                lparam: lparam.0,
            },
            refused,
        );
    }
    let decision = if refused {
        "FALSE (refused)"
    } else {
        "TRUE (allowed)"
    };
    let action = format!(
        "WM_QUERYENDSESSION: {}; returning {decision}. Session outcome is not yet known.",
        session_message_details(wparam.0, lparam.0),
    );
    if refused {
        warn!("{action}");
    }
    logger::log_event(
        if refused {
            "session_end_refused"
        } else {
            "session_end_allowed"
        },
        EventSource::Shutdown,
        action,
        true,
    );
    LRESULT(isize::from(!refused))
}

fn is_shutdown_or_session_end_in_progress() -> bool {
    unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) != 0 }
}

fn should_run_end_session_cleanup(wparam: WPARAM, shutdown_in_progress: bool) -> bool {
    wparam.0 != 0 && shutdown_in_progress
}

/// Registers the callback invoked when Windows forces session shutdown.
pub(crate) fn set_end_session_cleanup_callback(callback: fn()) {
    if let Ok(mut slot) = END_SESSION_CLEANUP_CALLBACK.lock() {
        *slot = Some(callback);
    }
    END_SESSION_CLEANUP_STARTED.store(false, Ordering::Release);
}

/// Clears the forced-shutdown cleanup callback.
pub(crate) fn clear_end_session_cleanup_callback() {
    if let Ok(mut slot) = END_SESSION_CLEANUP_CALLBACK.lock() {
        *slot = None;
    }
    END_SESSION_CLEANUP_STARTED.store(false, Ordering::Release);
}

/// Registers the callback invoked for power broadcast suspend and resume notifications.
pub(crate) fn set_power_broadcast_callback(callback: fn(PowerBroadcastEvent)) {
    if let Ok(mut slot) = POWER_BROADCAST_CALLBACK.lock() {
        *slot = Some(callback);
    }
}

/// Clears the power broadcast callback.
pub(crate) fn clear_power_broadcast_callback() {
    if let Ok(mut slot) = POWER_BROADCAST_CALLBACK.lock() {
        *slot = None;
    }
}

fn handle_end_session(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let outcome = SESSION_DIAGNOSTICS
        .lock()
        .ok()
        .map(|mut diagnostics| diagnostics.end(hwnd.0 as isize, wparam.0 != 0))
        .unwrap_or(SessionOutcome {
            ending: wparam.0 != 0,
            refused_query: None,
        });
    if outcome.confirmed_cancellation_after_refusal() {
        super::record_blocked_event();
    }
    logger::log_event(
        outcome.event(),
        EventSource::Shutdown,
        outcome.action(wparam.0, lparam.0),
        !outcome.ending || outcome.refused_query.is_none(),
    );
    if should_run_end_session_cleanup(wparam, is_shutdown_or_session_end_in_progress()) {
        BLOCKER_ACTIVE.store(false, Ordering::Release);
        let _ = unsafe { ShutdownBlockReasonDestroy(hwnd) };

        if !END_SESSION_CLEANUP_STARTED.swap(true, Ordering::AcqRel) {
            if let Ok(slot) = END_SESSION_CLEANUP_CALLBACK.lock() {
                if let Some(callback) = *slot {
                    callback();
                }
            }
        }
    }

    LRESULT(0)
}

fn handle_power_broadcast(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let event = match wparam.0 as u32 {
        PBT_APMSUSPEND => Some(PowerBroadcastEvent::Suspend),
        PBT_APMRESUMEAUTOMATIC => Some(PowerBroadcastEvent::Resume),
        PBT_APMRESUMESUSPEND => Some(PowerBroadcastEvent::Resume),
        _ => None,
    };

    if let Some(event) = event {
        if let Ok(slot) = POWER_BROADCAST_CALLBACK.lock() {
            if let Some(callback) = *slot {
                callback(event);
            }
        }
        return LRESULT(1);
    }

    unsafe { DefWindowProcW(hwnd, WM_POWERBROADCAST, wparam, lparam) }
}

fn current_instance() -> WindowsResult<HINSTANCE> {
    unsafe { Ok(GetModuleHandleW(None)?.into()) }
}

fn register_suspend_resume_notification(window: HWND) -> WindowsResult<HPOWERNOTIFY> {
    unsafe { RegisterSuspendResumeNotification(HANDLE(window.0), DEVICE_NOTIFY_WINDOW_HANDLE) }
}

fn register_window_class(hinstance: HINSTANCE) -> WindowsResult<()> {
    if WINDOW_CLASS_REGISTERED.get().is_some() {
        return Ok(());
    }

    let class_definition = WNDCLASSW {
        hInstance: hinstance,
        lpszClassName: w!("WardoffLayer1WindowClass"),
        lpfnWndProc: Some(shutdown_window_proc),
        ..Default::default()
    };

    let atom = unsafe { RegisterClassW(&class_definition) };
    if atom == 0 {
        return Err(WindowsError::from_thread());
    }

    let _ = WINDOW_CLASS_REGISTERED.set(());
    Ok(())
}

fn create_message_window(hinstance: HINSTANCE) -> WindowsResult<HWND> {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("WardoffLayer1WindowClass"),
            w!("Wardoff Layer 1 Message Window"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance),
            None,
        )
    }
}

fn create_session_window(hinstance: HINSTANCE) -> WindowsResult<HWND> {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("WardoffLayer1WindowClass"),
            w!("Wardoff Layer 1 Session Window"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
}

fn destroy_window(window: HWND) {
    if window == HWND::default() {
        return;
    }

    unsafe {
        if IsWindow(Some(window)).as_bool() {
            let _ = DestroyWindow(window);
        }
    }
}

unsafe extern "system" fn shutdown_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_QUERYENDSESSION => handle_query_end_session(hwnd, wparam, lparam),
        WM_ENDSESSION => handle_end_session(hwnd, wparam, lparam),
        WM_POWERBROADCAST => handle_power_broadcast(hwnd, wparam, lparam),
        WM_DESTROY => {
            if let Ok(mut diagnostics) = SESSION_DIAGNOSTICS.lock() {
                let _ = diagnostics.forget_window(hwnd.0 as isize);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        session_message_details, should_run_end_session_cleanup, SessionDiagnostics, SessionQuery,
    };
    use windows::Win32::Foundation::WPARAM;

    #[test]
    fn end_session_cleanup_ignores_false_wparam() {
        assert!(!should_run_end_session_cleanup(WPARAM(0), false));
        assert!(!should_run_end_session_cleanup(WPARAM(0), true));
    }

    #[test]
    fn end_session_cleanup_ignores_spoof_like_end_session_without_shutdown_state() {
        assert!(!should_run_end_session_cleanup(WPARAM(1), false));
    }

    #[test]
    fn end_session_cleanup_runs_only_for_real_session_end() {
        assert!(should_run_end_session_cleanup(WPARAM(1), true));
    }

    fn refusal(window: isize) -> SessionQuery {
        SessionQuery {
            window,
            wparam: 17,
            lparam: 0xc0000001,
        }
    }

    #[test]
    fn refusal_alone_does_not_confirm_cancellation() {
        let mut diagnostics = SessionDiagnostics::new();
        diagnostics.query(refusal(1), true);
        assert_eq!(diagnostics.refusals, [refusal(1)]);
        let outcome = diagnostics.end(1, false);
        assert!(outcome.confirmed_cancellation_after_refusal());
        assert_eq!(outcome.event(), "session_end_cancelled");
        assert_eq!(outcome.refused_query, Some(refusal(1)));
        assert!(!diagnostics
            .end(1, false)
            .confirmed_cancellation_after_refusal());
    }

    #[test]
    fn session_can_end_despite_refusal_without_counting_a_block() {
        let mut diagnostics = SessionDiagnostics::new();
        diagnostics.query(refusal(1), true);
        let outcome = diagnostics.end(1, true);
        assert!(!outcome.confirmed_cancellation_after_refusal());
        assert_eq!(outcome.event(), "session_end_proceeding");
        assert_eq!(outcome.refused_query, Some(refusal(1)));
        assert!(!diagnostics
            .end(1, false)
            .confirmed_cancellation_after_refusal());
    }

    #[test]
    fn cancellation_without_a_refusal_is_not_counted() {
        assert!(!SessionDiagnostics::new()
            .end(1, false)
            .confirmed_cancellation_after_refusal());
    }

    #[test]
    fn allowed_query_discards_an_older_refusal() {
        let mut diagnostics = SessionDiagnostics::new();
        diagnostics.query(refusal(1), true);
        diagnostics.query(refusal(1), false);
        assert!(!diagnostics
            .end(1, false)
            .confirmed_cancellation_after_refusal());
    }

    #[test]
    fn repeated_queries_count_only_one_confirmed_cancellation() {
        let mut diagnostics = SessionDiagnostics::new();
        diagnostics.query(refusal(1), true);
        diagnostics.query(refusal(1), true);
        assert!(diagnostics
            .end(1, false)
            .confirmed_cancellation_after_refusal());
        assert!(!diagnostics
            .end(1, false)
            .confirmed_cancellation_after_refusal());
    }

    #[test]
    fn window_outcomes_cannot_confirm_another_windows_refusal() {
        let mut diagnostics = SessionDiagnostics::new();
        diagnostics.query(refusal(1), true);
        diagnostics.query(refusal(2), true);
        assert!(!diagnostics
            .end(3, false)
            .confirmed_cancellation_after_refusal());
        assert!(diagnostics
            .end(2, false)
            .confirmed_cancellation_after_refusal());
        assert!(diagnostics
            .end(1, false)
            .confirmed_cancellation_after_refusal());
    }

    #[test]
    fn destroying_a_window_discards_its_pending_diagnostic() {
        let mut diagnostics = SessionDiagnostics::new();
        diagnostics.query(refusal(1), true);
        let _ = diagnostics.forget_window(1);
        assert!(!diagnostics
            .end(1, false)
            .confirmed_cancellation_after_refusal());
    }

    #[test]
    fn message_details_preserve_parameters_and_combined_unknown_flags() {
        let details = session_message_details(17, 0xc0000003);
        assert!(details.contains("wParam=17, lParam=0xc0000003"));
        assert!(details.contains("logoff=true, critical=true, close_app=true"));
        let details = session_message_details(0, 0);
        assert!(details.contains("logoff=false, critical=false, close_app=false"));
    }

    #[test]
    fn outcome_preserves_distinct_query_and_end_parameters() {
        let mut diagnostics = SessionDiagnostics::new();
        diagnostics.query(refusal(1), true);
        let outcome = diagnostics.end(1, true);
        let action = outcome.action(42, 0xc0000003);
        assert!(action.contains("WM_ENDSESSION: wParam=42, lParam=0xc0000003"));
        assert!(action.contains("wParam=17, lParam=0xc0000001"));
        assert!(action.contains("is ending"));
        assert!(action.contains("initiator and cancellation cause are unknown"));
        assert!(!outcome.confirmed_cancellation_after_refusal());
    }

    #[test]
    fn query_handler_preserves_refusal_and_allow_without_incrementing_counter() {
        use std::sync::atomic::Ordering;
        use windows::Win32::Foundation::{HWND, LPARAM};

        let window = HWND(123_usize as *mut _);
        let before = super::super::BLOCKED_EVENT_COUNT.load(Ordering::Relaxed);
        let was_active = super::BLOCKER_ACTIVE.swap(true, Ordering::AcqRel);
        let refused = super::handle_query_end_session(window, WPARAM(17), LPARAM(0xc0000001));
        super::BLOCKER_ACTIVE.store(false, Ordering::Release);
        let allowed = super::handle_query_end_session(window, WPARAM(0), LPARAM(0));
        super::BLOCKER_ACTIVE.store(was_active, Ordering::Release);
        let after = super::super::BLOCKED_EVENT_COUNT.load(Ordering::Relaxed);

        assert_eq!(refused.0, 0);
        assert_eq!(allowed.0, 1);
        assert_eq!(before, after);
    }
}
