//! Pure decisions shared by startup and the running application.
//!
//! This module maps CLI intent and tray state without performing Windows effects.
//! Keep handles, threads, logging and Task Scheduler access in their existing owners.

use crate::blocker::{BlockerMode, PowerAction};
use crate::cli::RequestedAction;
use crate::ipc::{IpcRequest, PipeMode};
use crate::power_intent::PowerIntent;

pub(crate) mod sessions;

/// Bounds the next native wait by the next maintenance deadline.
pub(crate) fn session_maintenance_wait_ms(needed: bool, next_due: u64, now: u64) -> Option<u32> {
    needed.then(|| next_due.saturating_sub(now).min(1000) as u32)
}

/// Resolves retry effects from current intentions, including a tray suspend pause.
pub(crate) fn session_protection_target(
    ledger: &sessions::SessionLedger,
    wake_paused: bool,
) -> Option<PowerIntent> {
    if wake_paused {
        None
    } else {
        ledger.effective_intent()
    }
}

/// Keeps indefinite Block and global stop available alongside optional sessions.
pub(crate) fn tray_intent_presentation(
    mode: BlockerMode,
    intent: PowerIntent,
    manual: Option<PowerIntent>,
    stoppable: bool,
) -> TrayModePresentation {
    let mut presentation = tray_mode_presentation(mode, intent);
    if mode == BlockerMode::Block && manual != Some(PowerIntent::SystemAndDisplay) {
        presentation.block_enabled = true;
        presentation.block_text = if manual.is_none() {
            "Block indefinitely"
        } else {
            "Block (keep display on)"
        };
    }
    presentation.allow_enabled |= stoppable;
    presentation
}

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
    /// Idle-power request set selected for Block mode.
    pub(crate) power_intent: PowerIntent,
}

/// The mode-dependent tray labels and enabled actions shown by the primary runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TrayModePresentation {
    pub(crate) tooltip: &'static str,
    pub(crate) block_text: &'static str,
    pub(crate) block_enabled: bool,
    pub(crate) allow_enabled: bool,
}

/// Selects the tray presentation for the current mode and idle-power intent.
pub(crate) fn tray_mode_presentation(
    mode: BlockerMode,
    intent: PowerIntent,
) -> TrayModePresentation {
    match mode {
        BlockerMode::Block if intent == PowerIntent::SystemOnly => TrayModePresentation {
            tooltip: "Wardoff - Block mode (display timeout allowed)",
            block_text: "Block (keep display on)",
            block_enabled: true,
            allow_enabled: true,
        },
        BlockerMode::Block => TrayModePresentation {
            tooltip: "Wardoff - Block mode",
            block_text: "Block",
            block_enabled: false,
            allow_enabled: true,
        },
        BlockerMode::Allow => TrayModePresentation {
            tooltip: "Wardoff - Allow mode",
            block_text: "Block",
            block_enabled: true,
            allow_enabled: false,
        },
    }
}

/// Tracks a tray suspend request that should restore Block after resume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingWakeRestore {
    /// A tray action queued restoration before a suspend notification arrived.
    PendingSuspend {
        action: PowerAction,
        power_intent: PowerIntent,
    },
    /// Windows reported suspend; restoration still waits for resume.
    WaitingForResume {
        action: PowerAction,
        power_intent: PowerIntent,
    },
}

impl PendingWakeRestore {
    /// Returns the power action whose Block state should be restored.
    pub(crate) fn action(self) -> PowerAction {
        match self {
            PendingWakeRestore::PendingSuspend { action, .. }
            | PendingWakeRestore::WaitingForResume { action, .. } => action,
        }
    }

    /// Returns the exact request set to reacquire after the tray action.
    pub(crate) fn power_intent(self) -> PowerIntent {
        match self {
            PendingWakeRestore::PendingSuspend { power_intent, .. }
            | PendingWakeRestore::WaitingForResume { power_intent, .. } => power_intent,
        }
    }

    /// Records a suspend notification without changing the requested action.
    pub(crate) fn mark_system_suspended(self) -> Self {
        PendingWakeRestore::WaitingForResume {
            action: self.action(),
            power_intent: self.power_intent(),
        }
    }
}

/// Maps runtime-launching actions; read-only and autostart actions must be dispatched earlier.
pub(crate) fn runtime_options_for(action: RequestedAction) -> RuntimeOptions {
    match action {
        RequestedAction::Default => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Visible,
            power_intent: PowerIntent::SystemAndDisplay,
        },
        RequestedAction::Block => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Headless,
            power_intent: PowerIntent::SystemAndDisplay,
        },
        RequestedAction::BlockWithPowerIntent { intent } => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Headless,
            power_intent: intent,
        },
        RequestedAction::Allow => RuntimeOptions {
            initial_mode: BlockerMode::Allow,
            tray_surface: TraySurface::Visible,
            power_intent: PowerIntent::SystemAndDisplay,
        },
        RequestedAction::Hide => RuntimeOptions {
            initial_mode: BlockerMode::Block,
            tray_surface: TraySurface::Hidden,
            power_intent: PowerIntent::SystemAndDisplay,
        },
        RequestedAction::Autostart { .. } => {
            unreachable!("autostart changes do not start the primary runtime")
        }
        RequestedAction::Status
        | RequestedAction::Log { .. }
        | RequestedAction::Doctor { .. }
        | RequestedAction::DiagnosticsProbe => {
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
        RequestedAction::BlockWithPowerIntent { intent } => IpcRequest::SetPowerIntent { intent },
        RequestedAction::Allow => IpcRequest::SetMode {
            mode: PipeMode::Allow,
        },
        RequestedAction::Autostart { enabled } => IpcRequest::SetAutostart { enabled },
        RequestedAction::Status
        | RequestedAction::Log { .. }
        | RequestedAction::Doctor { .. }
        | RequestedAction::DiagnosticsProbe => {
            unreachable!("read-only CLI actions do not use the control IPC request helper")
        }
    }
}

/// Adds restart guidance only for a legacy server rejecting the new command.
pub(crate) fn control_error_message(request: &IpcRequest, message: String) -> String {
    let session_command = match request {
        IpcRequest::BeginSession => Some("begin_session"),
        IpcRequest::AcquireSession { .. } => Some("acquire_session"),
        IpcRequest::ExtendSession { .. } => Some("extend_session"),
        IpcRequest::ReleaseSession { .. } => Some("release_session"),
        _ => None,
    };
    if session_command
        .is_some_and(|command| message.contains(&format!("unknown variant `{command}`")))
    {
        return "The running Wardoff instance does not support protection sessions. Restart Wardoff with this executable, then retry.".to_string();
    }
    if matches!(request, IpcRequest::SetPowerIntent { .. })
        && message.contains("unknown variant")
        && message.contains("set_power_intent")
    {
        return "The running Wardoff instance does not support --display-timeout. Restart Wardoff with this executable, then retry.".to_string();
    }
    message
}

/// Queues wake restoration only when a Block-mode tray action requests suspend.
pub(crate) fn pending_wake_restore_for_tray_power_action(
    previous_mode: BlockerMode,
    previous_intent: PowerIntent,
    action: PowerAction,
) -> Option<PendingWakeRestore> {
    match (previous_mode, action) {
        (BlockerMode::Block, PowerAction::Sleep) => Some(PendingWakeRestore::PendingSuspend {
            action: PowerAction::Sleep,
            power_intent: previous_intent,
        }),
        (BlockerMode::Block, PowerAction::Hibernate) => Some(PendingWakeRestore::PendingSuspend {
            action: PowerAction::Hibernate,
            power_intent: previous_intent,
        }),
        _ => None,
    }
}

/// Accepts resume even if Windows did not deliver the earlier suspend notification.
pub(crate) fn pending_wake_restore_ready_for_resume(
    pending_restore: Option<PendingWakeRestore>,
) -> Option<(PowerAction, PowerIntent)> {
    // Windows can deliver a resume notification to this runtime without an earlier
    // suspend broadcast reaching the hidden window, so any queued tray wake-restore
    // must still restore Block mode on resume even if Wardoff never observed suspend.
    match pending_restore {
        Some(PendingWakeRestore::PendingSuspend {
            action,
            power_intent,
        })
        | Some(PendingWakeRestore::WaitingForResume {
            action,
            power_intent,
        }) => Some((action, power_intent)),
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
    #[test]
    fn session_maintenance_wait_cannot_postpone_an_already_near_deadline() {
        use super::session_maintenance_wait_ms;
        assert_eq!(session_maintenance_wait_ms(false, 150, 100), None);
        assert_eq!(session_maintenance_wait_ms(true, 150, 100), Some(50));
        assert_eq!(session_maintenance_wait_ms(true, 100, 150), Some(0));
        assert_eq!(session_maintenance_wait_ms(true, u64::MAX, 0), Some(1000));
    }
    use super::{session_protection_target, sessions, tray_intent_presentation};

    #[test]
    fn session_cleanup_retry_uses_new_manual_or_session_intent() {
        use sessions::{OwnerKey, SessionLedger, SessionSpec};
        let mut ledger = SessionLedger::new([1; 16], None);
        // A previous Allow cleanup failed. Its target must not be cached.
        assert_eq!(session_protection_target(&ledger, false), None);
        ledger.set_manual(Some(PowerIntent::SystemAndDisplay));
        assert_eq!(
            session_protection_target(&ledger, false),
            Some(PowerIntent::SystemAndDisplay)
        );
        ledger.global_stop();
        assert_eq!(session_protection_target(&ledger, false), None);
        let owner = OwnerKey {
            pid: 42,
            creation_time: 9,
        };
        let key = ledger.begin(owner, 100).unwrap();
        ledger
            .acquire(
                owner,
                key,
                SessionSpec {
                    duration_ms: 100,
                    intent: PowerIntent::SystemOnly,
                    reason: "Fixture work".into(),
                },
                100,
            )
            .unwrap();
        assert_eq!(
            session_protection_target(&ledger, false),
            Some(PowerIntent::SystemOnly)
        );
    }

    #[test]
    fn session_suspend_expiry_cannot_restore_a_stale_power_intent() {
        use sessions::{OwnerKey, SessionLedger, SessionSpec};
        let mut ledger = SessionLedger::new([1; 16], Some(PowerIntent::SystemOnly));
        let owner = OwnerKey {
            pid: 42,
            creation_time: 9,
        };
        let key = ledger.begin(owner, 100).unwrap();
        ledger
            .acquire(
                owner,
                key,
                SessionSpec {
                    duration_ms: 100,
                    intent: PowerIntent::SystemAndDisplay,
                    reason: "Fixture presentation".into(),
                },
                100,
            )
            .unwrap();
        assert_eq!(session_protection_target(&ledger, true), None);
        assert_eq!(
            session_protection_target(&ledger, false),
            Some(PowerIntent::SystemAndDisplay)
        );
        ledger.expire(5000, |_| true);
        assert_eq!(
            session_protection_target(&ledger, false),
            Some(PowerIntent::SystemOnly)
        );
        ledger.global_stop();
        assert_eq!(session_protection_target(&ledger, false), None);
    }

    #[test]
    fn session_tray_keeps_manual_block_and_pending_allow_available() {
        let presentation = tray_intent_presentation(
            BlockerMode::Block,
            PowerIntent::SystemAndDisplay,
            None,
            true,
        );
        assert!(presentation.block_enabled);
        assert_eq!(presentation.block_text, "Block indefinitely");
        let paused = tray_intent_presentation(
            BlockerMode::Allow,
            PowerIntent::SystemAndDisplay,
            Some(PowerIntent::SystemAndDisplay),
            true,
        );
        assert!(paused.allow_enabled);
        // A temporary full-pair contributor must not hide the action that
        // changes an existing system-only manual choice to indefinite full-pair.
        let temporary_display = tray_intent_presentation(
            BlockerMode::Block,
            PowerIntent::SystemAndDisplay,
            Some(PowerIntent::SystemOnly),
            true,
        );
        assert!(temporary_display.block_enabled);
        assert_eq!(temporary_display.block_text, "Block (keep display on)");
    }

    #[test]
    fn session_legacy_rejection_has_restart_guidance_without_changing_other_errors() {
        let request = IpcRequest::BeginSession;
        assert!(
            control_error_message(&request, "unknown variant `begin_session`".into())
                .to_lowercase()
                .contains("restart")
        );
        assert_eq!(
            control_error_message(&request, "access denied".into()),
            "access denied"
        );
        assert_eq!(
            control_error_message(&request, "unknown variant `other`".into()),
            "unknown variant `other`"
        );
    }

    use super::{
        control_error_message, ipc_request_for, pending_wake_restore_for_tray_power_action,
        pending_wake_restore_ready_for_resume, resolved_autostart_state_for_tray,
        runtime_options_for, tray_mode_presentation, BlockerMode, IpcRequest, PendingWakeRestore,
        PipeMode, PowerAction, PowerIntent, RequestedAction, RuntimeOptions, TrayModePresentation,
        TraySurface,
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
                    tray_surface,
                    power_intent: PowerIntent::SystemAndDisplay,
                }
            );
        }
        for intent in [PowerIntent::SystemOnly, PowerIntent::SystemAndDisplay] {
            assert_eq!(
                runtime_options_for(RequestedAction::BlockWithPowerIntent { intent }),
                RuntimeOptions {
                    initial_mode: BlockerMode::Block,
                    tray_surface: TraySurface::Headless,
                    power_intent: intent,
                }
            );
        }
    }

    #[test]
    fn tray_presentation_reflects_selected_display_timeout_intent() {
        assert_eq!(
            tray_mode_presentation(BlockerMode::Block, PowerIntent::SystemOnly),
            TrayModePresentation {
                tooltip: "Wardoff - Block mode (display timeout allowed)",
                block_text: "Block (keep display on)",
                block_enabled: true,
                allow_enabled: true,
            }
        );
        assert_eq!(
            tray_mode_presentation(BlockerMode::Block, PowerIntent::SystemAndDisplay),
            TrayModePresentation {
                tooltip: "Wardoff - Block mode",
                block_text: "Block",
                block_enabled: false,
                allow_enabled: true,
            }
        );
        for intent in [PowerIntent::SystemOnly, PowerIntent::SystemAndDisplay] {
            assert_eq!(
                tray_mode_presentation(BlockerMode::Allow, intent),
                TrayModePresentation {
                    tooltip: "Wardoff - Allow mode",
                    block_text: "Block",
                    block_enabled: true,
                    allow_enabled: false,
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
        for intent in [PowerIntent::SystemOnly, PowerIntent::SystemAndDisplay] {
            assert!(matches!(
                ipc_request_for(RequestedAction::BlockWithPowerIntent { intent }),
                IpcRequest::SetPowerIntent { intent: actual }
                    if actual == intent
            ));
        }
        for enabled in [false, true] {
            assert!(
                matches!(ipc_request_for(RequestedAction::Autostart { enabled }), IpcRequest::SetAutostart { enabled: actual } if actual == enabled)
            );
        }
    }

    #[test]
    fn only_legacy_unknown_power_intent_errors_get_restart_guidance() {
        let request = IpcRequest::SetPowerIntent {
            intent: PowerIntent::SystemOnly,
        };
        let message =
            control_error_message(&request, "unknown variant `set_power_intent`".to_string());
        assert!(message.to_ascii_lowercase().contains("restart"));
        assert_eq!(
            control_error_message(&request, "access denied".to_string()),
            "access denied"
        );
        let mode = IpcRequest::SetMode {
            mode: PipeMode::Allow,
        };
        assert_eq!(
            control_error_message(&mode, "unknown variant `set_power_intent`".to_string()),
            "unknown variant `set_power_intent`"
        );
        assert_eq!(
            control_error_message(&request, "unknown variant `other_command`".to_string()),
            "unknown variant `other_command`"
        );
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
            pending_wake_restore_for_tray_power_action(
                BlockerMode::Block,
                PowerIntent::SystemOnly,
                PowerAction::Sleep
            ),
            Some(PendingWakeRestore::PendingSuspend {
                action: PowerAction::Sleep,
                power_intent: PowerIntent::SystemOnly
            })
        );
        assert_eq!(
            pending_wake_restore_for_tray_power_action(
                BlockerMode::Block,
                PowerIntent::SystemAndDisplay,
                PowerAction::Hibernate
            ),
            Some(PendingWakeRestore::PendingSuspend {
                action: PowerAction::Hibernate,
                power_intent: PowerIntent::SystemAndDisplay
            })
        );
        assert_eq!(
            pending_wake_restore_for_tray_power_action(
                BlockerMode::Allow,
                PowerIntent::SystemOnly,
                PowerAction::Sleep
            ),
            None
        );
        assert_eq!(
            pending_wake_restore_for_tray_power_action(
                BlockerMode::Block,
                PowerIntent::SystemOnly,
                PowerAction::Shutdown
            ),
            None
        );
    }

    #[test]
    fn pending_restore_marks_suspend_and_preserves_requested_action() {
        let pending = PendingWakeRestore::PendingSuspend {
            action: PowerAction::Hibernate,
            power_intent: PowerIntent::SystemOnly,
        };
        assert_eq!(
            pending.mark_system_suspended(),
            PendingWakeRestore::WaitingForResume {
                action: PowerAction::Hibernate,
                power_intent: PowerIntent::SystemOnly
            }
        );
        assert_eq!(pending.action(), PowerAction::Hibernate);
    }

    #[test]
    fn resume_restores_block_mode_even_if_suspend_was_not_observed() {
        assert_eq!(
            pending_wake_restore_ready_for_resume(Some(PendingWakeRestore::PendingSuspend {
                action: PowerAction::Sleep,
                power_intent: PowerIntent::SystemOnly
            })),
            Some((PowerAction::Sleep, PowerIntent::SystemOnly))
        );
        assert_eq!(
            pending_wake_restore_ready_for_resume(Some(PendingWakeRestore::WaitingForResume {
                action: PowerAction::Sleep,
                power_intent: PowerIntent::SystemAndDisplay
            })),
            Some((PowerAction::Sleep, PowerIntent::SystemAndDisplay))
        );
    }

    #[test]
    fn pending_restore_preserves_each_action_intent_and_pending_state() {
        for action in [PowerAction::Sleep, PowerAction::Hibernate] {
            for intent in [PowerIntent::SystemOnly, PowerIntent::SystemAndDisplay] {
                for pending in [
                    PendingWakeRestore::PendingSuspend {
                        action,
                        power_intent: intent,
                    },
                    PendingWakeRestore::WaitingForResume {
                        action,
                        power_intent: intent,
                    },
                ] {
                    assert_eq!(pending.action(), action);
                    assert_eq!(pending.power_intent(), intent);
                    assert_eq!(
                        pending_wake_restore_ready_for_resume(Some(pending)),
                        Some((action, intent))
                    );
                    assert_eq!(
                        pending.mark_system_suspended(),
                        PendingWakeRestore::WaitingForResume {
                            action,
                            power_intent: intent
                        }
                    );
                }
                assert_eq!(
                    pending_wake_restore_for_tray_power_action(BlockerMode::Block, intent, action),
                    Some(PendingWakeRestore::PendingSuspend {
                        action,
                        power_intent: intent
                    })
                );
            }
        }
        assert_eq!(pending_wake_restore_ready_for_resume(None), None);
        assert_eq!(
            pending_wake_restore_for_tray_power_action(
                BlockerMode::Allow,
                PowerIntent::SystemOnly,
                PowerAction::Sleep
            ),
            None
        );
    }
}
