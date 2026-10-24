use crate::diagnostics::{
    LayerId, LayerSnapshot, Observation, PowerRequests, RequestObservation, RuntimeSnapshot,
};
use crate::power_intent::PowerIntent;
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

/// Shared AbortSystemShutdownW(None) handling used by Layers 2 and 4.
pub mod abort;
/// Scaffolding for Layer 2 local shutdown.exe handling.
pub mod local;
pub(crate) mod observation;
/// Scaffolding for Layer 4 remote shutdown abort handling.
pub mod remote;
/// Scaffolding for Layer 1 interactive shutdown blocking.
pub mod shutdown;
/// Process-owned requests for idle-sleep and automatic display-timeout prevention.
pub mod sleep;
/// Scaffolding for Layer 3 UpdateOrchestrator reboot protection.
pub mod update;

/// Default text shown by Windows when Layer 1 blocks a shutdown.
pub const DEFAULT_SHUTDOWN_BLOCK_REASON: &str =
    "Wardoff is protecting this PC. Use the tray icon or run \"wardoff --allow\" before shutting down or signing out.";

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
    local_shutdown: bool,
    update: bool,
    remote: bool,
    /// Whether Wardoff acquired its request pair, not a guarantee against explicit sleep.
    sleep: bool,
}

impl LayerStatus {
    pub(crate) fn api_values(self) -> [bool; 5] {
        [
            self.shutdown,
            self.local_shutdown,
            self.update,
            self.remote,
            self.sleep,
        ]
    }
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
    power_intent: PowerIntent,
    shutdown_blocker: shutdown::ShutdownBlocker,
    local_shutdown_blocker: local::LocalShutdownBlocker,
    update_reboot_blocker: update::UpdateRebootBlocker,
    remote_shutdown_blocker: remote::RemoteShutdownBlocker,
    sleep_blocker: sleep::SleepBlocker,
    last_transition_failure: Option<LayerId>,
}

/// Creates the blocker coordinator that owns the layered shutdown guards.
pub fn create_blocker_coordinator(
    initial_mode: BlockerMode,
    power_intent: PowerIntent,
) -> Result<BlockerCoordinator, String> {
    BlockerCoordinator::new(DEFAULT_SHUTDOWN_BLOCK_REASON, initial_mode, power_intent)
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
    pub fn new(
        reason: &str,
        initial_mode: BlockerMode,
        power_intent: PowerIntent,
    ) -> Result<Self, String> {
        let shutdown_blocker = shutdown::ShutdownBlocker::new(reason)
            .map_err(|error| format!("Layer 1 could not create its shutdown windows: {error}"))?;

        let mut coordinator = Self {
            mode: BlockerMode::Allow,
            power_intent: PowerIntent::default(),
            shutdown_blocker,
            local_shutdown_blocker: local::LocalShutdownBlocker::default(),
            update_reboot_blocker: update::UpdateRebootBlocker::default(),
            remote_shutdown_blocker: remote::RemoteShutdownBlocker::default(),
            sleep_blocker: sleep::SleepBlocker::default(),
            last_transition_failure: None,
        };

        coordinator.set_configuration(initial_mode, power_intent)?;
        Ok(coordinator)
    }

    /// Returns the current application-wide blocker mode.
    pub fn mode(&self) -> BlockerMode {
        self.mode
    }

    /// Returns the selected request set, independently of current API activity.
    pub fn power_intent(&self) -> PowerIntent {
        self.power_intent
    }

    /// Returns whether each MVP blocker layer is currently active.
    pub(crate) fn capture_status(
        &self,
        elevated: Option<bool>,
        pending_wake_restore: bool,
    ) -> (LayerStatus, RuntimeSnapshot) {
        let (sleep, requests) = self.sleep_blocker.diagnostic_state();
        let observations = [
            if self.shutdown_blocker.is_active() {
                Observation::Active
            } else {
                Observation::Inactive
            },
            self.local_shutdown_blocker.observation(),
            self.update_reboot_blocker.observation(),
            self.remote_shutdown_blocker.observation(),
            sleep,
        ];
        let active = observations.map(|value| value.api_active() == Some(true));
        let legacy = LayerStatus {
            shutdown: active[0],
            local_shutdown: active[1],
            update: active[2],
            remote: active[3],
            sleep: active[4],
        };
        let requested = Some(self.mode == BlockerMode::Block);
        let display_requested =
            Some(self.mode == BlockerMode::Block && self.power_intent.request_mask()[1]);
        let snapshot = RuntimeSnapshot {
            schema_version: 1,
            elevated,
            manual_indefinite: self.mode == BlockerMode::Block || pending_wake_restore,
            pending_wake_restore,
            last_transition_failure: self.last_transition_failure,
            layers: crate::diagnostics::LAYER_IDS
                .into_iter()
                .zip(observations)
                .map(|(id, observation)| LayerSnapshot { id, observation })
                .collect(),
            power_requests: PowerRequests {
                system: RequestObservation {
                    requested,
                    api_active: Some(requests[0]),
                },
                display: RequestObservation {
                    requested: display_requested,
                    api_active: Some(requests[1]),
                },
            },
        };
        (legacy, snapshot)
    }

    /// Returns the number of shutdown attempts this runtime has actively blocked.
    pub fn blocked_count(&self) -> u64 {
        BLOCKED_EVENT_COUNT.load(Ordering::Relaxed)
    }

    /// Renews idle-power requests only when Block remains the requested mode.
    pub(crate) fn renew_sleep_requests_after_resume(&mut self) -> Result<(), String> {
        if self.mode == BlockerMode::Block {
            self.sleep_blocker.renew_after_resume(self.power_intent)?;
        }
        Ok(())
    }

    /// Disables the layered shutdown and sleep blocking scaffolding.
    pub fn deactivate(&mut self) -> Result<(), String> {
        self.set_mode(BlockerMode::Allow).map(|_| ())
    }

    /// Applies a mode command; explicit Block always restores the default pair.
    pub fn set_mode(&mut self, mode: BlockerMode) -> Result<bool, String> {
        let intent = if mode == BlockerMode::Block {
            PowerIntent::default()
        } else {
            self.power_intent
        };
        self.set_configuration(mode, intent)
    }

    /// Applies mode and intent together, preserving an active system request on flips.
    pub fn set_configuration(
        &mut self,
        mode: BlockerMode,
        intent: PowerIntent,
    ) -> Result<bool, String> {
        let configuration_changed = self.mode != mode || self.power_intent != intent;
        match mode {
            BlockerMode::Block if self.mode == BlockerMode::Block => {
                let was_active = self.sleep_blocker.is_active();
                if let Err(error) = self.sleep_blocker.activate(intent) {
                    self.last_transition_failure = Some(LayerId::IdlePower);
                    return Err(error);
                }
                if !configuration_changed && was_active {
                    return Ok(false);
                }
                self.last_transition_failure = None;
            }
            BlockerMode::Block => self.activate_block_mode(intent)?,
            BlockerMode::Allow if self.mode == BlockerMode::Allow => {
                self.power_intent = intent;
                return Ok(configuration_changed);
            }
            BlockerMode::Allow => self.activate_allow_mode()?,
        }

        self.mode = mode;
        self.power_intent = intent;
        info!(
            "Wardoff applied {} mode with {intent:?} idle-power intent at {}.",
            mode_label(self.mode),
            next_state_change_timestamp().to_rfc3339()
        );
        Ok(true)
    }

    /// Stops all blocker layers so the process can quit cleanly.
    pub fn shutdown(&mut self) -> Result<(), String> {
        self.deactivate()
    }

    /// Starts the non-blocking cleanup path used when Windows forces session shutdown.
    pub fn forced_shutdown_cleanup(&mut self) {
        self.update_reboot_blocker.begin_forced_shutdown_cleanup();
        self.sleep_blocker.begin_forced_shutdown_cleanup();
        self.remote_shutdown_blocker.begin_forced_shutdown_cleanup();
        self.mode = BlockerMode::Allow;
    }

    fn activate_block_mode(&mut self, intent: PowerIntent) -> Result<(), String> {
        let mut layer = LayerId::LocalShutdown;
        let activation_result = (|| {
            self.local_shutdown_blocker.activate()?;
            layer = LayerId::Update;
            self.update_reboot_blocker.activate()?;
            layer = LayerId::Remote;
            self.remote_shutdown_blocker.activate()?;
            layer = LayerId::IdlePower;
            self.sleep_blocker.activate(intent)?;
            layer = LayerId::Shutdown;
            self.shutdown_blocker
                .activate()
                .map_err(|error| format!("Layer 1 could not enter Block mode: {error}"))?;
            Ok(())
        })();

        if let Err(error) = activation_result {
            self.last_transition_failure = Some(layer);
            return Err(self.rollback_failed_block_activation(error));
        }
        self.last_transition_failure = None;
        Ok(())
    }

    fn activate_allow_mode(&mut self) -> Result<(), String> {
        deactivate_allow_mode_layers(
            || {
                self.shutdown_blocker
                    .deactivate()
                    .map_err(|error| error.to_string())
            },
            || self.sleep_blocker.deactivate(),
            || self.remote_shutdown_blocker.deactivate(),
            || self.update_reboot_blocker.deactivate(),
            || self.local_shutdown_blocker.deactivate(),
        )
    }

    fn rollback_failed_block_activation(&mut self, error: String) -> String {
        self.sleep_blocker.deactivate();
        self.remote_shutdown_blocker.deactivate();
        self.update_reboot_blocker.deactivate();
        self.local_shutdown_blocker.deactivate();

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

fn deactivate_allow_mode_layers<
    ShutdownDeactivate,
    SleepDeactivate,
    RemoteDeactivate,
    UpdateDeactivate,
    LocalDeactivate,
>(
    shutdown_deactivate: ShutdownDeactivate,
    sleep_deactivate: SleepDeactivate,
    remote_deactivate: RemoteDeactivate,
    update_deactivate: UpdateDeactivate,
    local_deactivate: LocalDeactivate,
) -> Result<(), String>
where
    ShutdownDeactivate: FnOnce() -> Result<(), String>,
    SleepDeactivate: FnOnce(),
    RemoteDeactivate: FnOnce(),
    UpdateDeactivate: FnOnce(),
    LocalDeactivate: FnOnce(),
{
    let shutdown_result = shutdown_deactivate();
    sleep_deactivate();
    remote_deactivate();
    update_deactivate();
    local_deactivate();

    shutdown_result.map_err(|error| {
        format!(
            "Wardoff could not fully switch to Allow mode because Layer 1 could not deactivate: {error}"
        )
    })
}

fn record_blocked_event() {
    BLOCKED_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::{deactivate_allow_mode_layers, LayerStatus};
    use std::cell::Cell;

    #[test]
    fn layer_status_serializes_the_layer2_flag() {
        let json = serde_json::to_string(&LayerStatus {
            shutdown: true,
            local_shutdown: false,
            update: true,
            remote: false,
            sleep: true,
        })
        .expect("LayerStatus should serialize");

        assert!(json.contains("\"local_shutdown\":false"));
    }

    #[test]
    fn allow_mode_cleanup_continues_after_layer1_error() {
        let sleep_deactivated = Cell::new(false);
        let remote_deactivated = Cell::new(false);
        let update_deactivated = Cell::new(false);
        let local_deactivated = Cell::new(false);

        let result = deactivate_allow_mode_layers(
            || Err("layer 1 destroy failed".to_string()),
            || sleep_deactivated.set(true),
            || remote_deactivated.set(true),
            || update_deactivated.set(true),
            || local_deactivated.set(true),
        );

        assert_eq!(
            result,
            Err(
                "Wardoff could not fully switch to Allow mode because Layer 1 could not deactivate: layer 1 destroy failed"
                    .to_string()
            )
        );
        assert!(sleep_deactivated.get());
        assert!(remote_deactivated.get());
        assert!(update_deactivated.get());
        assert!(local_deactivated.get());
    }
}
