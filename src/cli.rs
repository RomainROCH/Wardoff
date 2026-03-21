use clap::Parser;
use serde::Serialize;
use serde_json::Value;

/// Describes the high-level action requested through the Wardoff CLI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum RequestedAction {
    /// Enables blocking mode for supported shutdown and sleep paths.
    Block,
    /// Disables blocking mode and returns the system to its normal behavior.
    Allow,
    /// Emits machine-readable status output for scripting scenarios.
    Status,
    /// Starts the application without showing the tray surface immediately.
    Hide,
}

/// Defines the MVP command-line switches supported by Wardoff.
#[derive(Debug, Parser)]
#[command(
    author = "TheHaricover",
    version,
    about = "Open-source Windows shutdown/reboot/sleep blocker"
)]
pub struct WardoffCli {
    /// Enables blocking mode for the current session.
    #[arg(long)]
    block: bool,
    /// Disables blocking mode for the current session.
    #[arg(long)]
    allow: bool,
    /// Prints the current state as JSON.
    #[arg(long)]
    status: bool,
    /// Starts the application without showing the tray icon immediately.
    #[arg(long)]
    hide: bool,
}

/// Parses the current process arguments into the Wardoff CLI model.
pub fn parse_cli() -> WardoffCli {
    todo!("Parse command line arguments for the Wardoff CLI")
}

impl WardoffCli {
    /// Resolves the requested action from the parsed MVP CLI flags.
    pub fn requested_action(&self) -> RequestedAction {
        todo!("Determine the requested CLI action from the parsed flags")
    }

    /// Builds the JSON value template that will back `--status` output.
    pub fn status_template(&self) -> Value {
        todo!("Build the JSON payload template used by the --status command")
    }
}
