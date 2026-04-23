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
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::Threading::SetProcessShutdownParameters;
use windows::Win32::System::WindowsProgramming::SHUTDOWN_NORETRY;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IsWindow, RegisterClassW, ENDSESSION_LOGOFF,
    HWND_MESSAGE, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_DESTROY, WM_ENDSESSION, WM_POWERBROADCAST, WM_QUERYENDSESSION, WNDCLASSW,
    WS_OVERLAPPED,
};

static BLOCKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static WINDOW_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();
static END_SESSION_CLEANUP_CALLBACK: Mutex<Option<fn()>> = Mutex::new(None);
static END_SESSION_CLEANUP_STARTED: AtomicBool = AtomicBool::new(false);
static POWER_BROADCAST_CALLBACK: Mutex<Option<fn(PowerBroadcastEvent)>> = Mutex::new(None);

const SHUTDOWN_PRIORITY: u32 = 0x3FF;

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

        Ok(Self {
            message_window,
            session_window,
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
        destroy_window(self.session_window);
        destroy_window(self.message_window);
        BLOCKER_ACTIVE.store(false, Ordering::Release);
    }
}

/// Handles `WM_QUERYENDSESSION` for shutdown and sign-out while Wardoff is in blocking mode.
pub fn handle_query_end_session(_wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if BLOCKER_ACTIVE.load(Ordering::Acquire) {
        let requested_action = if is_logoff_query(lparam) {
            "sign-out"
        } else {
            "shutdown"
        };
        super::record_blocked_event();
        warn!(
            "Blocking WM_QUERYENDSESSION for {requested_action} while Layer 1 protection is active"
        );
        logger::log_event(
            "shutdown_blocked",
            EventSource::Shutdown,
            format!(
                "Layer 1 blocked a Windows {requested_action} request through WM_QUERYENDSESSION and returned FALSE."
            ),
            true,
        );
        return LRESULT(0);
    }

    LRESULT(1)
}

fn is_logoff_query(lparam: LPARAM) -> bool {
    (lparam.0 & ENDSESSION_LOGOFF as isize) != 0
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

fn handle_end_session(hwnd: HWND, wparam: WPARAM) -> LRESULT {
    if wparam.0 != 0 {
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
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => Some(PowerBroadcastEvent::Resume),
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
        WM_QUERYENDSESSION => handle_query_end_session(wparam, lparam),
        WM_ENDSESSION => handle_end_session(hwnd, wparam),
        WM_POWERBROADCAST => handle_power_broadcast(hwnd, wparam, lparam),
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
