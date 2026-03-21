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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use log::warn;
use windows::core::{w, Error as WindowsError, HSTRING, Result as WindowsResult};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::Threading::SetProcessShutdownParameters;
use windows::Win32::System::WindowsProgramming::SHUTDOWN_NORETRY;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    IsWindow, MSG, PostQuitMessage, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_DESTROY, WM_QUERYENDSESSION, WNDCLASSW, WS_OVERLAPPED,
};

static BLOCKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static WINDOW_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

const SHUTDOWN_PRIORITY: u32 = 0x3FF;

/// Owns the Layer 1 windows used for interactive shutdown blocking.
pub struct ShutdownBlocker {
    message_window: HWND,
    session_window: HWND,
}

impl ShutdownBlocker {
    /// Creates the Layer 1 blocker, including the requested message-only window
    /// and the hidden top-level window that actually receives shutdown queries.
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

        if let Err(error) = unsafe {
            SetProcessShutdownParameters(SHUTDOWN_PRIORITY, SHUTDOWN_NORETRY)
        } {
            destroy_window(session_window);
            destroy_window(message_window);
            return Err(error);
        }

        let reason = HSTRING::from(reason);
        if let Err(error) = unsafe { ShutdownBlockReasonCreate(session_window, &reason) } {
            destroy_window(session_window);
            destroy_window(message_window);
            return Err(error);
        }

        BLOCKER_ACTIVE.store(true, Ordering::Release);

        Ok(Self {
            message_window,
            session_window,
        })
    }
}

impl Drop for ShutdownBlocker {
    fn drop(&mut self) {
        if self.session_window != HWND::default() {
            unsafe {
                let _ = ShutdownBlockReasonDestroy(self.session_window);
            }
        }

        destroy_window(self.session_window);
        destroy_window(self.message_window);
        BLOCKER_ACTIVE.store(false, Ordering::Release);
    }
}

/// Creates the Layer 1 message-only companion window requested by the MVP plan.
#[allow(dead_code)]
pub fn create_shutdown_blocker_window() -> WindowsResult<HWND> {
    let hinstance = current_instance()?;
    register_window_class(hinstance)?;
    create_message_window(hinstance)
}

/// Handles `WM_QUERYENDSESSION` while Wardoff is in blocking mode.
pub fn handle_query_end_session(_wparam: WPARAM, _lparam: LPARAM) -> LRESULT {
    if BLOCKER_ACTIVE.load(Ordering::Acquire) {
        warn!("Blocking WM_QUERYENDSESSION while Layer 1 protection is active");
        return LRESULT(0);
    }

    LRESULT(1)
}

/// Returns the Windows message identifier used for interactive shutdown negotiation.
#[allow(dead_code)]
pub fn query_end_session_message() -> u32 {
    WM_QUERYENDSESSION
}

/// Runs the current thread's Windows message loop while Layer 1 is active.
pub fn run_message_loop() -> WindowsResult<()> {
    let mut message = MSG::default();

    loop {
        let get_message_result = unsafe { GetMessageW(&mut message, None, 0, 0) };

        match get_message_result.0 {
            -1 => return Err(WindowsError::from_thread()),
            0 => return Ok(()),
            _ => unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
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
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
