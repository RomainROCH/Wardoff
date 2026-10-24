use crate::blocker::{BlockerMode, LayerStatus};
use crate::diagnostics::{RuntimeMode, RuntimeSnapshot};
use crate::logger;
use clap::{Parser, Subcommand, ValueEnum};
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
    /// Explains observed protection without starting or elevating the runtime.
    Doctor { json: bool },
    /// Runs only the bounded, read-only local inspection child.
    DiagnosticsProbe,
}

impl RequestedAction {
    /// Returns whether this action is a read-only CLI path that must stay non-elevating.
    pub fn is_read_only(self) -> bool {
        matches!(
            self,
            Self::Status | Self::Log { .. } | Self::Doctor { .. } | Self::DiagnosticsProbe
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliAutostartState {
    On,
    Off,
}

#[derive(Debug, Subcommand)]
enum ReadOnlyCommand {
    /// Explains protection intent, observed activity and Windows limits.
    Doctor {
        /// Prints the versioned diagnostic report as JSON.
        #[arg(long)]
        json: bool,
    },
    #[command(name = "wardoff-diagnostics-probe", hide = true)]
    DiagnosticsProbe,
}

/// Defines the MVP command-line switches supported by Wardoff.
#[derive(Debug, Parser)]
#[command(
    name = "wardoff",
    author = "Romain ROCH",
    version = env!("CARGO_PKG_VERSION"),
    args_conflicts_with_subcommands = true,
    about = "Open-source Windows shutdown/reboot protection and idle-sleep prevention"
)]
pub struct WardoffCli {
    #[command(subcommand)]
    command: Option<ReadOnlyCommand>,
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
    /// Overrides the default number of structured log lines shown by `--log` (maximum 100000).
    #[arg(long, value_name = "N", requires = "log")]
    tail: Option<usize>,
    /// Internal marker used to prevent default-launch elevation loops.
    #[arg(long = "wardoff-elevated-relaunch", hide = true)]
    elevated_relaunch: bool,
    /// Internal marker used when Wardoff relaunches a long-lived runtime without a shell console.
    #[arg(long = "wardoff-detached-runtime", hide = true)]
    detached_runtime: bool,
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_snapshot"
    )]
    diagnostics: Option<RuntimeSnapshot>,
}

// Future or malformed extensions must not hide an otherwise readable legacy reply.
fn deserialize_snapshot<'de, D>(deserializer: D) -> Result<Option<RuntimeSnapshot>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

/// Parses the current process arguments into the Wardoff CLI model.
pub fn parse_cli() -> WardoffCli {
    WardoffCli::parse()
}

impl WardoffCli {
    /// Resolves the requested action from the parsed MVP CLI flags.
    pub fn requested_action(&self) -> RequestedAction {
        if let Some(command) = &self.command {
            return match command {
                ReadOnlyCommand::Doctor { json } => RequestedAction::Doctor { json: *json },
                ReadOnlyCommand::DiagnosticsProbe => RequestedAction::DiagnosticsProbe,
            };
        }
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

    /// Returns whether the current process was started by Wardoff's internal detached-runtime relaunch.
    pub fn is_internal_detached_runtime(&self) -> bool {
        self.detached_runtime
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
            diagnostics: None,
        }
    }

    /// Builds the inactive status payload returned when no primary instance exists.
    pub fn inactive() -> Self {
        Self {
            state: StatusState::Inactive,
            layers: None,
            uptime_seconds: None,
            blocked_count: None,
            diagnostics: None,
        }
    }

    /// Serializes the status payload into compact JSON for scripting scenarios.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let mut legacy = self.clone();
        legacy.diagnostics = None;
        serde_json::to_string(&legacy)
    }

    pub(crate) fn with_diagnostics(mut self, snapshot: RuntimeSnapshot) -> Self {
        self.diagnostics = Some(snapshot);
        self
    }

    pub(crate) fn diagnostic_input(
        self,
    ) -> (RuntimeMode, Option<[bool; 5]>, Option<RuntimeSnapshot>) {
        let mode = match self.state {
            StatusState::Inactive => RuntimeMode::Inactive,
            StatusState::Block => RuntimeMode::Block,
            StatusState::Allow => RuntimeMode::Allow,
        };
        (
            mode,
            self.layers.map(LayerStatus::api_values),
            self.diagnostics,
        )
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
    fn legacy_status_bytes_are_unchanged_with_an_internal_snapshot() {
        use crate::blocker::{BlockerMode, LayerStatus};
        use crate::diagnostics::{
            LayerSnapshot, Observation, PowerRequests, RequestObservation, RuntimeSnapshot,
            LAYER_IDS,
        };
        let expected = r#"{"state":"block","layers":{"shutdown":false,"local_shutdown":false,"update":false,"remote":false,"sleep":false},"uptime_seconds":42,"blocked_count":3}"#;
        let status = super::StatusOutput::active(BlockerMode::Block, LayerStatus::default(), 42, 3);
        assert_eq!(status.to_json().unwrap(), expected);
        let request = RequestObservation {
            requested: Some(true),
            api_active: Some(false),
        };
        let status = status.with_diagnostics(RuntimeSnapshot {
            schema_version: 1,
            elevated: Some(false),
            manual_indefinite: true,
            pending_wake_restore: false,
            last_transition_failure: None,
            layers: LAYER_IDS
                .into_iter()
                .map(|id| LayerSnapshot {
                    id,
                    observation: Observation::Starting,
                })
                .collect(),
            power_requests: PowerRequests {
                system: request.clone(),
                display: request,
            },
        });
        assert_eq!(status.to_json().unwrap(), expected);
        assert!(serde_json::to_value(&status)
            .unwrap()
            .get("diagnostics")
            .is_some());
        assert_eq!(
            super::StatusOutput::inactive().to_json().unwrap(),
            r#"{"state":"inactive"}"#
        );
        let parsed: super::StatusOutput =
            serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
        assert!(parsed.diagnostic_input().2.is_some());
    }

    #[test]
    fn unknown_or_malformed_snapshot_keeps_legacy_status_readable() {
        for extension in [
            r#"{"schema_version":99,"new_field":true}"#,
            r#"{"layers":null}"#,
            "null",
            "123",
        ] {
            let input = format!(r#"{{"state":"allow","diagnostics":{extension}}}"#);
            let status: super::StatusOutput = serde_json::from_str(&input).unwrap();
            assert_eq!(status.to_json().unwrap(), r#"{"state":"allow"}"#);
            assert!(status.diagnostic_input().2.is_none());
        }
    }

    #[test]
    fn doctor_and_probe_never_mix_with_runtime_flags() {
        for flag in ["--block", "--allow", "--hide", "--status", "--log"] {
            for command in ["doctor", "wardoff-diagnostics-probe"] {
                assert!(WardoffCli::try_parse_from(["wardoff", flag, command]).is_err());
                assert!(WardoffCli::try_parse_from(["wardoff", command, flag]).is_err());
            }
        }
        let probe = WardoffCli::try_parse_from(["wardoff", "wardoff-diagnostics-probe"]).unwrap();
        assert_eq!(probe.requested_action(), RequestedAction::DiagnosticsProbe);
        assert!(probe.requested_action().is_read_only());
    }

    #[test]
    fn default_launch_remains_default_when_relaunched_internally() {
        let cli = WardoffCli::parse_from([
            "wardoff",
            "--wardoff-elevated-relaunch",
            "--wardoff-detached-runtime",
        ]);

        assert_eq!(cli.requested_action(), RequestedAction::Default);
        assert!(cli.is_internal_elevated_relaunch());
        assert!(cli.is_internal_detached_runtime());
    }

    #[test]
    fn explicit_commands_are_not_reclassified_as_default_launches() {
        let cli = WardoffCli::parse_from(["wardoff", "--hide", "--wardoff-detached-runtime"]);

        assert_eq!(cli.requested_action(), RequestedAction::Hide);
        assert!(!cli.is_internal_elevated_relaunch());
        assert!(cli.is_internal_detached_runtime());
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
            .contains("Open-source Windows shutdown/reboot protection and idle-sleep prevention"));
        assert!(error
            .to_string()
            .contains(&format!("maximum {}", crate::logger::MAX_TAIL_LINE_COUNT)));
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

    #[test]
    fn doctor_subcommand_parses_as_read_only() {
        let result = WardoffCli::try_parse_from(["wardoff", "doctor"]);
        assert!(result.is_ok(), "doctor should be accepted: {result:?}");
        let cli = result.unwrap();

        assert!(cli.requested_action().is_read_only());
    }

    #[test]
    fn doctor_json_subcommand_parses_as_read_only() {
        let result = WardoffCli::try_parse_from(["wardoff", "doctor", "--json"]);
        assert!(
            result.is_ok(),
            "doctor --json should be accepted: {result:?}"
        );
        let cli = result.unwrap();

        assert!(cli.requested_action().is_read_only());
    }

    #[test]
    fn doctor_conflicts_with_existing_runtime_flags() {
        for arguments in [
            &["wardoff", "doctor", "--block"][..],
            &["wardoff", "doctor", "--allow"][..],
            &["wardoff", "doctor", "--hide"][..],
            &["wardoff", "doctor", "--status"][..],
            &["wardoff", "doctor", "--log"][..],
            &["wardoff", "doctor", "--autostart", "on"][..],
        ] {
            let result = WardoffCli::try_parse_from(arguments);
            assert!(result.is_err(), "doctor should conflict with {arguments:?}");
        }
    }

    #[test]
    fn doctor_json_flag_requires_doctor_subcommand() {
        let result = WardoffCli::try_parse_from(["wardoff", "--json"]);

        assert!(result.is_err(), "--json without doctor should be rejected");
    }
}
