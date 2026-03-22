use chrono::{DateTime, Utc};
use log::info;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use windows::core::{Error as WindowsError, Result as WindowsResult};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::{
    ExitWindowsEx, EWX_REBOOT, EWX_SHUTDOWN, SHTDN_REASON_FLAG_PLANNED,
    SHTDN_REASON_MAJOR_APPLICATION, SHTDN_REASON_MINOR_OTHER, SHUTDOWN_REASON,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Scaffolding for Layer 4 remote shutdown abort handling.
pub mod remote;
/// Scaffolding for Layer 1 interactive shutdown blocking.
pub mod shutdown;
/// Scaffolding for SetThreadExecutionState-based sleep blocking.
pub mod sleep;
/// Scaffolding for Layer 3 UpdateOrchestrator reboot protection.
pub mod update;

/// Default text shown by Windows when Layer 1 blocks a shutdown.
pub const DEFAULT_SHUTDOWN_BLOCK_REASON: &str =
    "Wardoff is blocking shutdown while Layer 1 protection is active.";

static BLOCKED_EVENT_COUNT: AtomicU64 = AtomicU64::new(0);

/// Represents the global blocker mode exposed by the Wardoff core.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BlockerMode {
    /// Actively blocks supported shutdown, reboot, and sleep paths.
    Block,
    /// Allows supported shutdown, reboot, and sleep paths to proceed.
    Allow,
}

/// Represents whether each MVP blocker layer is currently active.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LayerStatus {
    shutdown: bool,
    update: bool,
    remote: bool,
    sleep: bool,
}

/// Represents the system power actions exposed by the MVP tray menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerAction {
    /// Starts a standard Windows shutdown flow.
    Shutdown,
    /// Starts a standard Windows reboot flow.
    Reboot,
    /// Requests standard sleep through the Windows power API.
    Sleep,
    /// Requests hibernate through the Windows power API.
    Hibernate,
}

/// Coordinates the blocker sub-systems used by the MVP runtime.
pub struct BlockerCoordinator {
    mode: BlockerMode,
    shutdown_blocker: shutdown::ShutdownBlocker,
    update_reboot_blocker: update::UpdateRebootBlocker,
    remote_shutdown_blocker: remote::RemoteShutdownBlocker,
    sleep_blocker: sleep::SleepBlocker,
}

/// Creates the blocker coordinator that owns the layered shutdown guards.
pub fn create_blocker_coordinator(initial_mode: BlockerMode) -> Result<BlockerCoordinator, String> {
    BlockerCoordinator::new(DEFAULT_SHUTDOWN_BLOCK_REASON, initial_mode)
}

/// Returns the timestamp associated with the next blocker state transition.
pub fn next_state_change_timestamp() -> DateTime<Utc> {
    Utc::now()
}

/// Executes a Windows power action after Wardoff has switched to Allow mode.
pub fn execute_power_action(action: PowerAction) -> Result<(), String> {
    enable_shutdown_privilege().map_err(|error| {
        format!("Wardoff could not enable the shutdown privilege before {action:?}: {error}")
    })?;

    match action {
        PowerAction::Shutdown => unsafe { ExitWindowsEx(EWX_SHUTDOWN, planned_shutdown_reason()) }
            .map_err(|error| format!("Windows rejected the shutdown request: {error}")),
        PowerAction::Reboot => unsafe { ExitWindowsEx(EWX_REBOOT, planned_shutdown_reason()) }
            .map_err(|error| format!("Windows rejected the reboot request: {error}")),
        PowerAction::Sleep => request_suspend(false)
            .map_err(|error| format!("Windows rejected the sleep request: {error}")),
        PowerAction::Hibernate => request_suspend(true)
            .map_err(|error| format!("Windows rejected the hibernate request: {error}")),
    }
}

impl BlockerCoordinator {
    /// Creates the blocker coordinator and starts Wardoff in Block mode.
    pub fn new(reason: &str, initial_mode: BlockerMode) -> Result<Self, String> {
        let shutdown_blocker = shutdown::ShutdownBlocker::new(reason)
            .map_err(|error| format!("Layer 1 could not create its shutdown windows: {error}"))?;

        let mut coordinator = Self {
            mode: BlockerMode::Allow,
            shutdown_blocker,
            update_reboot_blocker: update::UpdateRebootBlocker::default(),
            remote_shutdown_blocker: remote::RemoteShutdownBlocker::default(),
            sleep_blocker: sleep::SleepBlocker::default(),
        };

        coordinator.set_mode(initial_mode)?;
        Ok(coordinator)
    }

    /// Returns the current application-wide blocker mode.
    pub fn mode(&self) -> BlockerMode {
        self.mode
    }

    /// Returns whether each MVP blocker layer is currently active.
    pub fn layer_status(&self) -> LayerStatus {
        LayerStatus {
            shutdown: self.shutdown_blocker.is_active(),
            update: self.update_reboot_blocker.is_active(),
            remote: self.remote_shutdown_blocker.is_active(),
            sleep: self.sleep_blocker.is_active(),
        }
    }

    /// Returns the number of shutdown attempts this runtime has actively blocked.
    pub fn blocked_count(&self) -> u64 {
        BLOCKED_EVENT_COUNT.load(Ordering::Relaxed)
    }

    /// Disables the layered shutdown and sleep blocking scaffolding.
    pub fn deactivate(&mut self) -> Result<(), String> {
        self.set_mode(BlockerMode::Allow).map(|_| ())
    }

    /// Applies a new global blocker mode and starts or stops all MVP layers.
    pub fn set_mode(&mut self, mode: BlockerMode) -> Result<bool, String> {
        if self.mode == mode {
            return Ok(false);
        }

        match mode {
            BlockerMode::Block => self.activate_block_mode()?,
            BlockerMode::Allow => self.activate_allow_mode()?,
        }

        self.mode = mode;
        info!(
            "Wardoff switched to {} mode at {}.",
            mode_label(self.mode),
            next_state_change_timestamp().to_rfc3339()
        );
        Ok(true)
    }

    /// Stops all blocker layers so the process can quit cleanly.
    pub fn shutdown(&mut self) -> Result<(), String> {
        self.deactivate()
    }

    fn activate_block_mode(&mut self) -> Result<(), String> {
        let activation_result = (|| {
            self.update_reboot_blocker.activate()?;
            self.remote_shutdown_blocker.activate()?;
            self.sleep_blocker.activate()?;
            self.shutdown_blocker
                .activate()
                .map_err(|error| format!("Layer 1 could not enter Block mode: {error}"))?;
            Ok(())
        })();

        if let Err(error) = activation_result {
            return Err(self.rollback_failed_block_activation(error));
        }

        Ok(())
    }

    fn activate_allow_mode(&mut self) -> Result<(), String> {
        self.shutdown_blocker
            .deactivate()
            .map_err(|error| format!("Layer 1 could not enter Allow mode: {error}"))?;
        self.sleep_blocker.deactivate();
        self.remote_shutdown_blocker.deactivate();
        self.update_reboot_blocker.deactivate();
        Ok(())
    }

    fn rollback_failed_block_activation(&mut self, error: String) -> String {
        self.sleep_blocker.deactivate();
        self.remote_shutdown_blocker.deactivate();
        self.update_reboot_blocker.deactivate();

        match self.shutdown_blocker.deactivate() {
            Ok(()) => format!(
                "Wardoff could not switch to Block mode: {error}. Wardoff rolled back safely to Allow mode."
            ),
            Err(rollback_error) => format!(
                "Wardoff could not switch to Block mode: {error}. Wardoff also failed to roll Layer 1 back to Allow mode: {rollback_error}"
            ),
        }
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

fn enable_shutdown_privilege() -> WindowsResult<()> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )?;
        let token = HandleGuard(token);

        let mut shutdown_luid = LUID::default();
        LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &mut shutdown_luid)?;

        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: shutdown_luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        AdjustTokenPrivileges(
            token.0,
            false,
            Some(&privileges as *const TOKEN_PRIVILEGES),
            0,
            None,
            None,
        )?;

        if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
            return Err(WindowsError::from_thread());
        }

        Ok(())
    }
}

fn planned_shutdown_reason() -> SHUTDOWN_REASON {
    SHTDN_REASON_MAJOR_APPLICATION | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED
}

fn request_suspend(hibernate: bool) -> Result<(), String> {
    if unsafe { SetSuspendState(hibernate, false, false) } {
        Ok(())
    } else {
        Err(WindowsError::from_thread().to_string())
    }
}

fn mode_label(mode: BlockerMode) -> &'static str {
    match mode {
        BlockerMode::Block => "Block",
        BlockerMode::Allow => "Allow",
    }
}

fn record_blocked_event() {
    BLOCKED_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
}
