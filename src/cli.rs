use crate::blocker::{BlockerMode, LayerStatus};
use crate::logger;
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

/// Describes the high-level action requested through the Wardoff CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestedAction {
    /// Starts the primary runtime in Block mode with the tray visible.
    Default,
    /// Starts the primary runtime in Block mode without any tray icon.
    Block,
    /// Switches an existing instance to Allow mode or starts a new one in Allow mode.
    Allow,
    /// Emits machine-readable status output for scripting scenarios.
    Status,
    /// Starts the primary runtime in Block mode with the tray icon hidden.
    Hide,
    /// Creates, updates, or removes the current-user autostart task.
    Autostart { enabled: bool },
    /// Prints the newest structured log lines without starting the runtime.
    Log { tail: usize },
}

impl RequestedAction {
    /// Returns whether this action is a read-only CLI path that must stay non-elevating.
    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Status | Self::Log { .. })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliAutostartState {
    On,
    Off,
}

/// Defines the MVP command-line switches supported by Wardoff.
#[derive(Debug, Parser)]
#[command(
    name = "wardoff",
    author = "Romain ROCH",
    version = env!("CARGO_PKG_VERSION"),
    about = "Open-source Windows shutdown/reboot/sleep blocker"
)]
pub struct WardoffCli {
    /// Enables blocking mode for the current session without showing a tray icon.
    #[arg(long, conflicts_with_all = ["allow", "status", "autostart", "log"])]
    block: bool,
    /// Disables blocking mode for the current session.
    #[arg(
        long,
        conflicts_with_all = ["block", "status", "hide", "autostart", "log"]
    )]
    allow: bool,
    /// Prints the current state as JSON.
    #[arg(
        long,
        conflicts_with_all = ["block", "allow", "hide", "autostart", "log"]
    )]
    status: bool,
    /// Starts the application in Block mode with a hidden tray icon.
    #[arg(long, conflicts_with_all = ["allow", "status", "autostart", "log"])]
    hide: bool,
    /// Creates or removes the current-user autostart scheduled task.
    #[arg(
        long,
        value_enum,
        value_name = "on|off",
        conflicts_with_all = ["block", "allow", "status", "hide", "log"]
    )]
    autostart: Option<CliAutostartState>,
    /// Prints the newest structured Wardoff JSON log lines.
    #[arg(long, conflicts_with_all = ["block", "allow", "status", "hide", "autostart"])]
    log: bool,
    /// Overrides the default number of structured log lines shown by `--log`.
    #[arg(long, value_name = "N", requires = "log")]
    tail: Option<usize>,
    /// Internal marker used to prevent default-launch elevation loops.
    #[arg(long = "wardoff-elevated-relaunch", hide = true)]
    elevated_relaunch: bool,
}

/// Represents the serialized state string returned by `wardoff --status`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusState {
    /// The runtime is actively blocking supported shutdown paths.
    Block,
    /// The runtime is allowing supported shutdown paths.
    Allow,
    /// No primary Wardoff runtime is currently active.
    Inactive,
}

/// Represents the JSON payload returned by `wardoff --status`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StatusOutput {
    state: StatusState,
    #[serde(skip_serializing_if = "Option::is_none")]
    layers: Option<LayerStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uptime_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocked_count: Option<u64>,
}

/// Parses the current process arguments into the Wardoff CLI model.
pub fn parse_cli() -> WardoffCli {
    WardoffCli::parse()
}

impl WardoffCli {
    /// Resolves the requested action from the parsed MVP CLI flags.
    pub fn requested_action(&self) -> RequestedAction {
        if self.allow {
            RequestedAction::Allow
        } else if self.status {
            RequestedAction::Status
        } else if self.hide {
            RequestedAction::Hide
        } else if self.block {
            RequestedAction::Block
        } else if let Some(autostart) = self.autostart {
            RequestedAction::Autostart {
                enabled: matches!(autostart, CliAutostartState::On),
            }
        } else if self.log {
            RequestedAction::Log {
                tail: self.tail.unwrap_or_else(logger::default_tail_line_count),
            }
        } else {
            RequestedAction::Default
        }
    }

    /// Returns whether the current process was started by Wardoff's internal elevation relaunch.
    pub fn is_internal_elevated_relaunch(&self) -> bool {
        self.elevated_relaunch
    }
}

impl StatusOutput {
    /// Builds the active status payload returned while a primary instance is running.
    pub fn active(
        mode: BlockerMode,
        layers: LayerStatus,
        uptime_seconds: u64,
        blocked_count: u64,
    ) -> Self {
        Self {
            state: mode.into(),
            layers: Some(layers),
            uptime_seconds: Some(uptime_seconds),
            blocked_count: Some(blocked_count),
        }
    }

    /// Builds the inactive status payload returned when no primary instance exists.
    pub fn inactive() -> Self {
        Self {
            state: StatusState::Inactive,
            layers: None,
            uptime_seconds: None,
            blocked_count: None,
        }
    }

    /// Serializes the status payload into compact JSON for scripting scenarios.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl From<BlockerMode> for StatusState {
    fn from(mode: BlockerMode) -> Self {
        match mode {
            BlockerMode::Block => StatusState::Block,
            BlockerMode::Allow => StatusState::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RequestedAction, WardoffCli};
    use clap::{error::ErrorKind, Parser};

    #[test]
    fn default_launch_remains_default_when_relaunched_internally() {
        let cli = WardoffCli::parse_from(["wardoff", "--wardoff-elevated-relaunch"]);

        assert_eq!(cli.requested_action(), RequestedAction::Default);
        assert!(cli.is_internal_elevated_relaunch());
    }

    #[test]
    fn explicit_commands_are_not_reclassified_as_default_launches() {
        let cli = WardoffCli::parse_from(["wardoff", "--hide"]);

        assert_eq!(cli.requested_action(), RequestedAction::Hide);
        assert!(!cli.is_internal_elevated_relaunch());
    }

    #[test]
    fn version_flag_reports_the_expected_binary_name_and_package_version() {
        let error = WardoffCli::try_parse_from(["wardoff", "--version"])
            .expect_err("--version should short-circuit argument parsing");

        assert_eq!(error.kind(), ErrorKind::DisplayVersion);
        assert_eq!(
            error.to_string().trim(),
            format!("wardoff {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn help_flag_short_circuits_argument_parsing() {
        let error = WardoffCli::try_parse_from(["wardoff", "--help"])
            .expect_err("--help should short-circuit argument parsing");

        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        assert!(error
            .to_string()
            .contains("Open-source Windows shutdown/reboot/sleep blocker"));
    }

    #[test]
    fn read_only_actions_are_classified_explicitly() {
        let status = WardoffCli::parse_from(["wardoff", "--status"]);
        let log = WardoffCli::parse_from(["wardoff", "--log", "--tail", "3"]);
        let default_launch = WardoffCli::parse_from(["wardoff"]);
        let autostart = WardoffCli::parse_from(["wardoff", "--autostart", "on"]);

        assert!(status.requested_action().is_read_only());
        assert!(log.requested_action().is_read_only());
        assert!(!default_launch.requested_action().is_read_only());
        assert!(!autostart.requested_action().is_read_only());
    }
}
