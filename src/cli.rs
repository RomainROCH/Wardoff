use crate::blocker::{BlockerMode, LayerStatus};
use crate::logger;
use clap::{ArgGroup, Parser};
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
    /// Prints the newest structured log lines without starting the runtime.
    Log { tail: usize },
}

/// Defines the MVP command-line switches supported by Wardoff.
#[derive(Debug, Parser)]
#[command(
    author = "TheHaricover",
    version,
    about = "Open-source Windows shutdown/reboot/sleep blocker"
)]
#[command(group(
    ArgGroup::new("requested-action")
        .args(["block", "allow", "status", "hide", "log"])
        .multiple(false)
))]
pub struct WardoffCli {
    /// Enables blocking mode for the current session without showing a tray icon.
    #[arg(long, group = "requested-action")]
    block: bool,
    /// Disables blocking mode for the current session.
    #[arg(long, group = "requested-action")]
    allow: bool,
    /// Prints the current state as JSON.
    #[arg(long, group = "requested-action")]
    status: bool,
    /// Starts the application with a hidden tray icon.
    #[arg(long, group = "requested-action")]
    hide: bool,
    /// Prints the newest structured Wardoff JSON log lines.
    #[arg(long, group = "requested-action")]
    log: bool,
    /// Overrides the default number of structured log lines shown by `--log`.
    #[arg(long, value_name = "N", requires = "log")]
    tail: Option<usize>,
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
        if self.block {
            RequestedAction::Block
        } else if self.allow {
            RequestedAction::Allow
        } else if self.status {
            RequestedAction::Status
        } else if self.hide {
            RequestedAction::Hide
        } else if self.log {
            RequestedAction::Log {
                tail: self.tail.unwrap_or_else(logger::default_tail_line_count),
            }
        } else {
            RequestedAction::Default
        }
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
