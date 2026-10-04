//! Pure decisions shared by startup and the running application.
//!
//! This module maps CLI intent and tray state without performing Windows effects.
//! Keep handles, threads, logging and Task Scheduler access in their existing owners.

use crate::blocker::{BlockerMode, PowerAction};
use crate::cli::RequestedAction;
use crate::ipc::{IpcRequest, PipeMode};

/// Selects whether startup creates a visible, hidden or absent tray surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraySurface {
    /// Creates the normal visible tray icon.
    Visible,
    /// Keeps a tray service whose icon starts hidden.
    Hidden,
    /// Starts without a tray service.
    Headless,
}

/// Describes primary startup without starting a runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeOptions {
    /// Mode to acquire during primary bootstrap.
    pub(crate) initial_mode: BlockerMode,
    /// Tray service and initial visibility to create.
    pub(crate) tray_surface: TraySurface,
}

/// Tracks a tray suspend request that should restore Block after resume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingWakeRestore {
    /// A tray action queued restoration before a suspend notification arrived.
    PendingSuspend(PowerAction),
    /// Windows reported suspend; restoration still waits for resume.
    WaitingForResume(PowerAction),
}

impl PendingWakeRestore {
    /// Returns the power action whose Block state should be restored.
    pub(crate) fn action(self) -> PowerAction {
        match self {
            PendingWakeRestore::PendingSuspend(action)
            | PendingWakeRestore::WaitingForResume(action) => action,
        }
    }

    /// Records a suspend notification without changing the requested action.
    pub(crate) fn mark_system_suspended(self) -> Self {
        PendingWakeRestore::WaitingForResume(self.action())
    }
}

/// Maps runtime-launching actions; read-only and autostart actions must be dispatched earlier.
pub(crate) fn runtime_options_for(action: RequestedAction) -> RuntimeOptions {
    match action {
        RequestedAction::Default => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Visible,
        },
        RequestedAction::Block => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Headless,
        },
        RequestedAction::Allow => RuntimeOptions {
            initial_mode: BlockerMode::Allow,
            tray_surface: TraySurface::Visible,
        },
        RequestedAction::Hide => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Hidden,
        },
        RequestedAction::Autostart { .. } => {
            unreachable!("autostart changes do not start the primary runtime")
        }
        RequestedAction::Status | RequestedAction::Log { .. } => {
            unreachable!("read-only CLI actions never start the runtime")
        }
    }
}

/// Maps secondary control actions; read-only actions must be dispatched earlier.
pub(crate) fn ipc_request_for(action: RequestedAction) -> IpcRequest {
    match action {
        RequestedAction::Default | RequestedAction::Block | RequestedAction::Hide => {
            IpcRequest::SetMode {
                mode: PipeMode::Block,
            }
        }
        RequestedAction::Allow => IpcRequest::SetMode {
            mode: PipeMode::Allow,
        },
        RequestedAction::Autostart { enabled } => IpcRequest::SetAutostart { enabled },
        RequestedAction::Status | RequestedAction::Log { .. } => {
            unreachable!("read-only CLI actions do not use the control IPC request helper")
        }
    }
}

/// Queues wake restoration only when a Block-mode tray action requests suspend.
pub(crate) fn pending_wake_restore_for_tray_power_action(
    previous_mode: BlockerMode,
    action: PowerAction,
) -> Option<PendingWakeRestore> {
    match (previous_mode, action) {
        (BlockerMode::Block, PowerAction::Sleep) => {
            Some(PendingWakeRestore::PendingSuspend(PowerAction::Sleep))
        }
        (BlockerMode::Block, PowerAction::Hibernate) => {
            Some(PendingWakeRestore::PendingSuspend(PowerAction::Hibernate))
        }
        _ => None,
    }
}

/// Accepts resume even if Windows did not deliver the earlier suspend notification.
pub(crate) fn pending_wake_restore_ready_for_resume(
    pending_restore: Option<PendingWakeRestore>,
) -> Option<PowerAction> {
    // Windows can deliver a resume notification to this runtime without an earlier
    // suspend broadcast reaching the hidden window, so any queued tray wake-restore
    // must still restore Block mode on resume even if Wardoff never observed suspend.
    match pending_restore {
        Some(PendingWakeRestore::PendingSuspend(action))
        | Some(PendingWakeRestore::WaitingForResume(action)) => Some(action),
        _ => None,
    }
}

/// Prefers the current task state, retaining the previous checkbox on a read failure.
pub(crate) fn resolved_autostart_state_for_tray(
    previous_state: Option<bool>,
    actual_state: Result<bool, String>,
) -> Result<bool, String> {
    match actual_state {
        Ok(actual_state) => Ok(actual_state),
        Err(error) => previous_state.ok_or(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ipc_request_for, pending_wake_restore_for_tray_power_action,
        pending_wake_restore_ready_for_resume, resolved_autostart_state_for_tray,
        runtime_options_for, BlockerMode, IpcRequest, PendingWakeRestore, PipeMode, PowerAction,
        RequestedAction, RuntimeOptions, TraySurface,
    };

    #[test]
    fn startup_preserves_visible_hidden_and_headless_modes() {
        for (action, initial_mode, tray_surface) in [
            (
                RequestedAction::Default,
                BlockerMode::Block,
                TraySurface::Visible,
            ),
            (
                RequestedAction::Block,
                BlockerMode::Block,
                TraySurface::Headless,
            ),
            (
                RequestedAction::Allow,
                BlockerMode::Allow,
                TraySurface::Visible,
            ),
            (
                RequestedAction::Hide,
                BlockerMode::Block,
                TraySurface::Hidden,
            ),
        ] {
            assert_eq!(
                runtime_options_for(action),
                RuntimeOptions {
                    initial_mode,
                    tray_surface
                }
            );
        }
    }

    #[test]
    fn secondary_commands_preserve_control_protocol_mapping() {
        for action in [
            RequestedAction::Default,
            RequestedAction::Block,
            RequestedAction::Hide,
        ] {
            assert!(matches!(
                ipc_request_for(action),
                IpcRequest::SetMode {
                    mode: PipeMode::Block
                }
            ));
        }
        assert!(matches!(
            ipc_request_for(RequestedAction::Allow),
            IpcRequest::SetMode {
                mode: PipeMode::Allow
            }
        ));
        for enabled in [false, true] {
            assert!(
                matches!(ipc_request_for(RequestedAction::Autostart { enabled }), IpcRequest::SetAutostart { enabled: actual } if actual == enabled)
            );
        }
    }

    #[test]
    fn prefers_current_autostart_state_when_available() {
        assert_eq!(
            resolved_autostart_state_for_tray(Some(false), Ok(true)),
            Ok(true)
        );
    }

    #[test]
    fn falls_back_to_previous_autostart_state_when_reread_fails() {
        assert_eq!(
            resolved_autostart_state_for_tray(Some(false), Err("read failed".to_string())),
            Ok(false)
        );
    }

    #[test]
    fn preserves_unknown_autostart_state_when_reads_fail() {
        assert_eq!(
            resolved_autostart_state_for_tray(None, Err("read failed".to_string())),
            Err("read failed".to_string())
        );
    }

    #[test]
    fn only_block_mode_sleep_and_hibernate_queue_restore_after_wake() {
        assert_eq!(
            pending_wake_restore_for_tray_power_action(BlockerMode::Block, PowerAction::Sleep),
            Some(PendingWakeRestore::PendingSuspend(PowerAction::Sleep))
        );
        assert_eq!(
            pending_wake_restore_for_tray_power_action(BlockerMode::Block, PowerAction::Hibernate),
            Some(PendingWakeRestore::PendingSuspend(PowerAction::Hibernate))
        );
        assert_eq!(
            pending_wake_restore_for_tray_power_action(BlockerMode::Allow, PowerAction::Sleep),
            None
        );
        assert_eq!(
            pending_wake_restore_for_tray_power_action(BlockerMode::Block, PowerAction::Shutdown),
            None
        );
    }

    #[test]
    fn pending_restore_marks_suspend_and_preserves_requested_action() {
        let pending = PendingWakeRestore::PendingSuspend(PowerAction::Hibernate);
        assert_eq!(
            pending.mark_system_suspended(),
            PendingWakeRestore::WaitingForResume(PowerAction::Hibernate)
        );
        assert_eq!(pending.action(), PowerAction::Hibernate);
    }

    #[test]
    fn resume_restores_block_mode_even_if_suspend_was_not_observed() {
        assert_eq!(
            pending_wake_restore_ready_for_resume(Some(PendingWakeRestore::PendingSuspend(
                PowerAction::Sleep
            ))),
            Some(PowerAction::Sleep)
        );
        assert_eq!(
            pending_wake_restore_ready_for_resume(Some(PendingWakeRestore::WaitingForResume(
                PowerAction::Sleep
            ))),
            Some(PowerAction::Sleep)
        );
    }
}
