use crate::blocker::BlockerMode;
use crate::logger::{self, EventSource};
use log::info;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder,
};
use windows::core::{w, Error as WindowsError, Result as WindowsResult};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, IsWindow, PeekMessageW,
    RegisterClassW, RegisterWindowMessageW, TranslateMessage, MSG, PM_NOREMOVE, PM_REMOVE,
    WINDOW_EX_STYLE, WM_DESTROY, WM_QUIT, WNDCLASSW, WS_OVERLAPPED,
};

/// Scaffolding for tray icon asset selection.
pub mod icon;

const TRAY_THREAD_NAME: &str = "wardoff-tray";
const TRAY_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const TRAY_UNAVAILABLE_WARNING_INTERVAL: Duration = Duration::from_secs(30);
const TRAY_THREAD_POLL_INTERVAL: Duration = Duration::from_millis(200);
const TASKBAR_CREATED_WINDOW_CLASS_NAME: windows::core::PCWSTR = w!("WardoffTrayTaskbarCreatedWindow");

static TASKBAR_CREATED_WINDOW_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();
static TASKBAR_CREATED_MESSAGE_ID: OnceLock<u32> = OnceLock::new();
static TASKBAR_CREATED_PENDING: AtomicBool = AtomicBool::new(false);

/// Controls whether the primary runtime shows or hides its tray icon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayVisibility {
    /// Creates the tray icon in its normal visible state.
    Visible,
    /// Creates the tray icon but immediately hides it.
    Hidden,
}

/// Represents the actions available from the MVP tray menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayAction {
    /// Switches the application into blocking mode.
    Block,
    /// Switches the application into allow mode.
    Allow,
    /// Changes whether Wardoff starts automatically at current-user logon.
    SetAutostart(bool),
    /// Starts a standard shutdown flow from the tray.
    Shutdown,
    /// Starts a standard reboot flow from the tray.
    Reboot,
    /// Requests sleep from the tray menu.
    Sleep,
    /// Requests hibernate from the tray menu.
    Hibernate,
    /// Terminates the Wardoff process.
    Quit,
}

/// Sends state updates to the background tray thread and receives tray menu actions.
pub(crate) struct TrayServiceHandle {
    action_rx: Receiver<TrayAction>,
    command_tx: Sender<TrayCommand>,
    join_handle: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrayCommand {
    SetMode(BlockerMode),
    SetAutostart(bool),
    Shutdown,
}

/// Coordinates tray creation and menu event dispatch.
pub struct TrayController {
    tray_icon: Option<TrayIcon>,
    visibility: TrayVisibility,
    block_item: MenuItem,
    allow_item: MenuItem,
    autostart_item: CheckMenuItem,
    shutdown_item: MenuItem,
    reboot_item: MenuItem,
    sleep_item: MenuItem,
    hibernate_item: MenuItem,
    quit_item: MenuItem,
    autostart_enabled: Cell<bool>,
}

/// Creates the tray controller used by the background application surface.
pub fn create_tray_controller(visibility: TrayVisibility) -> Result<TrayController, String> {
    TrayController::new(visibility)
}

/// Starts the tray surface on its own Win32 message-loop thread.
pub(crate) fn spawn_tray_service(
    visibility: TrayVisibility,
    initial_mode: BlockerMode,
) -> Result<TrayServiceHandle, String> {
    let (action_tx, action_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();

    let join_handle = thread::Builder::new()
        .name(TRAY_THREAD_NAME.to_string())
        .spawn(move || run_tray_thread(visibility, initial_mode, command_rx, action_tx))
        .map_err(|error| format!("Wardoff could not start its tray thread: {error}"))?;

    Ok(TrayServiceHandle {
        action_rx,
        command_tx,
        join_handle: Some(join_handle),
    })
}

impl TrayController {
    /// Creates the MVP tray icon with the required right-click menu actions.
    pub fn new(visibility: TrayVisibility) -> Result<Self, String> {
        let tray_menu = Menu::new();

        let block_item = MenuItem::with_id("wardoff.block", "Block", true, None);
        let allow_item = MenuItem::with_id("wardoff.allow", "Allow", true, None);
        let autostart_item =
            CheckMenuItem::with_id("wardoff.autostart", "Start with Windows", true, false, None);
        let shutdown_item = MenuItem::with_id("wardoff.shutdown", "Shutdown", true, None);
        let reboot_item = MenuItem::with_id("wardoff.reboot", "Reboot", true, None);
        let sleep_item = MenuItem::with_id("wardoff.sleep", "Sleep", true, None);
        let hibernate_item = MenuItem::with_id("wardoff.hibernate", "Hibernate", true, None);
        let quit_item = MenuItem::with_id("wardoff.quit", "Quit", true, None);
        let separator_one = PredefinedMenuItem::separator();
        let separator_two = PredefinedMenuItem::separator();

        tray_menu
            .append_items(&[
                &block_item,
                &allow_item,
                &autostart_item,
                &separator_one,
                &shutdown_item,
                &reboot_item,
                &sleep_item,
                &hibernate_item,
                &separator_two,
                &quit_item,
            ])
            .map_err(|error| format!("Wardoff could not assemble the tray menu: {error}"))?;

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_menu_on_left_click(false)
            .with_tooltip(tooltip_for_mode(BlockerMode::Block))
            .with_icon(icon::blocked_icon().map_err(|error| {
                format!("Wardoff could not build the blocked tray icon: {error}")
            })?)
            .build()
            .map_err(|error| format!("Wardoff could not create the tray icon: {error}"))?;

        let mut controller = Self {
            tray_icon: Some(tray_icon),
            visibility,
            block_item,
            allow_item,
            autostart_item,
            shutdown_item,
            reboot_item,
            sleep_item,
            hibernate_item,
            quit_item,
            autostart_enabled: Cell::new(false),
        };
        controller.set_mode(BlockerMode::Block)?;

        if visibility == TrayVisibility::Hidden {
            controller.hide_tray_icon()?;
        }

        logger::log_event(
            "tray_initialized",
            EventSource::Tray,
            format!(
                "Wardoff created a {} tray icon.",
                visibility_label(visibility)
            ),
            true,
        );

        Ok(controller)
    }

    /// Applies the current blocker mode to the tray icon, tooltip, and menu state.
    pub fn set_mode(&mut self, mode: BlockerMode) -> Result<(), String> {
        self.sync_menu_state(mode);

        if self.visibility == TrayVisibility::Hidden {
            // Windows can return E_FAIL when mutating icon metadata after the tray icon is hidden.
            return Ok(());
        }

        let icon = match mode {
            BlockerMode::Block => icon::blocked_icon().map_err(|error| {
                format!("Wardoff could not render the blocked tray icon: {error}")
            })?,
            BlockerMode::Allow => icon::allowed_icon().map_err(|error| {
                format!("Wardoff could not render the allowed tray icon: {error}")
            })?,
        };

        if let Some(tray_icon) = self.tray_icon.as_ref() {
            tray_icon
                .set_icon(Some(icon))
                .map_err(|error| format!("Wardoff could not update the tray icon: {error}"))?;
            tray_icon
                .set_tooltip(Some(tooltip_for_mode(mode)))
                .map_err(|error| format!("Wardoff could not update the tray tooltip: {error}"))?;
        }

        Ok(())
    }

    pub(crate) fn set_autostart_enabled(&self, enabled: bool) {
        self.autostart_enabled.set(enabled);
        self.autostart_item.set_checked(enabled);
    }

    /// Drains all pending tray menu activations received since the last message dispatch.
    pub fn drain_actions(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = self.action_for_menu_id(event.id()) {
                actions.push(action);
            }
        }

        actions
    }

    fn hide_tray_icon(&self) -> Result<(), String> {
        if let Some(tray_icon) = self.tray_icon.as_ref() {
            tray_icon
                .set_visible(false)
                .map_err(|error| format!("Wardoff could not hide the tray icon: {error}"))?;
        }

        logger::log_event(
            "tray_visibility_changed",
            EventSource::Tray,
            "Wardoff hid the tray icon.",
            true,
        );

        Ok(())
    }

    fn sync_menu_state(&self, mode: BlockerMode) {
        self.block_item.set_enabled(mode != BlockerMode::Block);
        self.allow_item.set_enabled(mode != BlockerMode::Allow);
    }

    fn action_for_menu_id(&self, id: &MenuId) -> Option<TrayAction> {
        if id == self.block_item.id() {
            Some(TrayAction::Block)
        } else if id == self.allow_item.id() {
            Some(TrayAction::Allow)
        } else if id == self.autostart_item.id() {
            Some(TrayAction::SetAutostart(!self.autostart_enabled.get()))
        } else if id == self.shutdown_item.id() {
            Some(TrayAction::Shutdown)
        } else if id == self.reboot_item.id() {
            Some(TrayAction::Reboot)
        } else if id == self.sleep_item.id() {
            Some(TrayAction::Sleep)
        } else if id == self.hibernate_item.id() {
            Some(TrayAction::Hibernate)
        } else if id == self.quit_item.id() {
            Some(TrayAction::Quit)
        } else {
            None
        }
    }
}

impl TrayServiceHandle {
    pub(crate) fn set_mode(&self, mode: BlockerMode) -> Result<(), String> {
        self.command_tx
            .send(TrayCommand::SetMode(mode))
            .map_err(|_| "Wardoff lost contact with its tray thread.".to_string())
    }

    pub(crate) fn set_autostart_enabled(&self, enabled: bool) -> Result<(), String> {
        self.command_tx
            .send(TrayCommand::SetAutostart(enabled))
            .map_err(|_| "Wardoff lost contact with its tray thread.".to_string())
    }

    pub(crate) fn drain_actions(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();

        while let Ok(action) = self.action_rx.try_recv() {
            actions.push(action);
        }

        actions
    }

    pub(crate) fn shutdown(&mut self) {
        let _ = self.command_tx.send(TrayCommand::Shutdown);

        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

struct TrayThreadState {
    visibility: TrayVisibility,
    mode: BlockerMode,
    autostart_enabled: bool,
    controller: Option<TrayController>,
    waiting_since: Option<Instant>,
    next_retry_at: Instant,
    last_warning_at: Option<Instant>,
}

fn run_tray_thread(
    visibility: TrayVisibility,
    initial_mode: BlockerMode,
    command_rx: Receiver<TrayCommand>,
    action_tx: Sender<TrayAction>,
) {
    ensure_message_queue();
    let _taskbar_created_window = create_taskbar_created_window()
        .map(TaskbarCreatedWindow)
        .map_err(|error| {
            log::warn!(
                "Wardoff could not create its Explorer restart listener window: {error}"
            );
            logger::log_event(
                "tray_taskbar_restart_listener_unavailable",
                EventSource::Tray,
                format!(
                    "Wardoff could not create its Explorer restart listener window: {error}"
                ),
                false,
            );
            error
        })
        .ok();

    let mut state = TrayThreadState {
        visibility,
        mode: initial_mode,
        autostart_enabled: false,
        controller: None,
        waiting_since: None,
        next_retry_at: Instant::now(),
        last_warning_at: None,
    };

    loop {
        pump_windows_messages();
        handle_taskbar_created(&mut state);
        forward_tray_actions(&state, &action_tx);

        while let Ok(command) = command_rx.try_recv() {
            if handle_tray_command(&mut state, command) {
                return;
            }
        }

        if state.controller.is_none() {
            attempt_tray_creation(&mut state);
        }

        match command_rx.recv_timeout(TRAY_THREAD_POLL_INTERVAL) {
            Ok(command) => {
                if handle_tray_command(&mut state, command) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

struct TaskbarCreatedWindow(HWND);

impl Drop for TaskbarCreatedWindow {
    fn drop(&mut self) {
        destroy_window(self.0);
    }
}

fn ensure_message_queue() {
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
}

fn pump_windows_messages() {
    let mut message = MSG::default();

    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        if message.message == WM_QUIT {
            continue;
        }

        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn handle_taskbar_created(state: &mut TrayThreadState) {
    if !TASKBAR_CREATED_PENDING.swap(false, Ordering::AcqRel) {
        return;
    }

    info!("Wardoff detected an Explorer restart and will recreate its tray icon.");
    logger::log_event(
        "tray_reinitialize_requested",
        EventSource::Tray,
        "Wardoff detected an Explorer restart and will recreate its tray icon.",
        true,
    );
    reset_tray_controller(state);
}

fn forward_tray_actions(state: &TrayThreadState, action_tx: &Sender<TrayAction>) {
    let Some(controller) = state.controller.as_ref() else {
        return;
    };

    for action in controller.drain_actions() {
        if action_tx.send(action).is_err() {
            break;
        }
    }
}

fn handle_tray_command(state: &mut TrayThreadState, command: TrayCommand) -> bool {
    match command {
        TrayCommand::SetMode(mode) => {
            state.mode = mode;
            if apply_current_tray_state(state).is_err() {
                reset_tray_controller(state);
            }
            false
        }
        TrayCommand::SetAutostart(enabled) => {
            state.autostart_enabled = enabled;
            if apply_current_tray_state(state).is_err() {
                reset_tray_controller(state);
            }
            false
        }
        TrayCommand::Shutdown => true,
    }
}

fn attempt_tray_creation(state: &mut TrayThreadState) {
    if state.waiting_since.is_none() {
        state.waiting_since = Some(Instant::now());
        state.last_warning_at = Some(Instant::now());
    }

    if Instant::now() >= state.next_retry_at {
        try_create_tray_controller(state);
    }

    if let Some(last_warning_at) = state.last_warning_at {
        if last_warning_at.elapsed() >= TRAY_UNAVAILABLE_WARNING_INTERVAL {
            log::warn!(
                "Wardoff is still waiting for Explorer to accept the tray icon; retrying every {} seconds.",
                TRAY_RETRY_INTERVAL.as_secs()
            );
            logger::log_event(
                "tray_retrying",
                EventSource::Tray,
                format!(
                    "Wardoff is still waiting for Explorer to accept the tray icon and will keep retrying every {} seconds.",
                    TRAY_RETRY_INTERVAL.as_secs()
                ),
                false,
            );
            state.last_warning_at = Some(Instant::now());
        }
    }
}

fn try_create_tray_controller(state: &mut TrayThreadState) {
    state.next_retry_at = Instant::now() + TRAY_RETRY_INTERVAL;

    match create_tray_controller(state.visibility) {
        Ok(mut controller) => {
            controller.set_autostart_enabled(state.autostart_enabled);
            match controller.set_mode(state.mode) {
                Ok(()) => {
                    if let Some(waiting_since) = state.waiting_since.take() {
                        if waiting_since.elapsed() >= TRAY_RETRY_INTERVAL {
                            info!("Wardoff created its tray icon after Explorer became ready.");
                            logger::log_event(
                                "tray_initialized",
                                EventSource::Tray,
                                "Wardoff created its tray icon after retrying until Explorer became ready.",
                                true,
                            );
                        }
                    }
                    state.last_warning_at = None;
                    state.controller = Some(controller);
                }
                Err(error) => {
                    log_tray_creation_failure(error);
                }
            }
        }
        Err(error) => {
            log_tray_creation_failure(error);
        }
    }
}

fn apply_current_tray_state(state: &mut TrayThreadState) -> Result<(), String> {
    let Some(controller) = state.controller.as_mut() else {
        return Ok(());
    };

    controller.set_autostart_enabled(state.autostart_enabled);
    controller.set_mode(state.mode)
}

fn reset_tray_controller(state: &mut TrayThreadState) {
    state.controller.take();
    if state.waiting_since.is_none() {
        state.waiting_since = Some(Instant::now());
    }
    if state.last_warning_at.is_none() {
        state.last_warning_at = Some(Instant::now());
    }
    state.next_retry_at = Instant::now();
}

fn log_tray_creation_failure(error: String) {
    let _ = error;
}

fn create_taskbar_created_window() -> WindowsResult<HWND> {
    let hinstance = current_instance()?;
    register_taskbar_created_window_class(hinstance)?;
    let _ = taskbar_created_message_id()?;

    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            TASKBAR_CREATED_WINDOW_CLASS_NAME,
            w!("Wardoff Tray TaskbarCreated Listener"),
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

fn current_instance() -> WindowsResult<HINSTANCE> {
    unsafe { Ok(GetModuleHandleW(None)?.into()) }
}

fn register_taskbar_created_window_class(hinstance: HINSTANCE) -> WindowsResult<()> {
    if TASKBAR_CREATED_WINDOW_CLASS_REGISTERED.get().is_some() {
        return Ok(());
    }

    let class_definition = WNDCLASSW {
        hInstance: hinstance,
        lpszClassName: TASKBAR_CREATED_WINDOW_CLASS_NAME,
        lpfnWndProc: Some(taskbar_created_window_proc),
        ..Default::default()
    };

    let atom = unsafe { RegisterClassW(&class_definition) };
    if atom == 0 {
        return Err(WindowsError::from_thread());
    }

    let _ = TASKBAR_CREATED_WINDOW_CLASS_REGISTERED.set(());
    Ok(())
}

fn taskbar_created_message_id() -> WindowsResult<u32> {
    if let Some(message_id) = TASKBAR_CREATED_MESSAGE_ID.get() {
        return Ok(*message_id);
    }

    let message_id = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    if message_id == 0 {
        return Err(WindowsError::from_thread());
    }

    let _ = TASKBAR_CREATED_MESSAGE_ID.set(message_id);
    Ok(message_id)
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

unsafe extern "system" fn taskbar_created_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if TASKBAR_CREATED_MESSAGE_ID
        .get()
        .is_some_and(|taskbar_created_message_id| message == *taskbar_created_message_id)
    {
        TASKBAR_CREATED_PENDING.store(true, Ordering::Release);
        return LRESULT(0);
    }

    match message {
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn tooltip_for_mode(mode: BlockerMode) -> &'static str {
    match mode {
        BlockerMode::Block => "Wardoff - Block mode",
        BlockerMode::Allow => "Wardoff - Allow mode",
    }
}

fn visibility_label(visibility: TrayVisibility) -> &'static str {
    match visibility {
        TrayVisibility::Visible => "visible",
        TrayVisibility::Hidden => "hidden",
    }
}
