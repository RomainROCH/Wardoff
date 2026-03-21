use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Describes how Wardoff should behave when the process starts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StartupMode {
    /// Starts in blocking mode.
    Block,
    /// Starts in allow mode.
    Allow,
}

/// Stores persisted application settings shared by CLI and tray flows.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    block_on_launch: bool,
    hide_tray_icon: bool,
    log_directory: PathBuf,
    startup_mode: StartupMode,
}

/// Stores log-related configuration for the rotating JSON lines sink.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoggingConfig {
    directory: PathBuf,
}

/// Returns the default configuration file path used by Wardoff.
pub fn config_path() -> PathBuf {
    todo!("Compute the default configuration file path for the Wardoff process")
}

/// Loads the persisted application configuration from disk or defaults.
pub fn load_config() -> AppConfig {
    todo!("Load persisted configuration or construct the initial defaults")
}

/// Returns the timestamp that should be associated with the next configuration snapshot.
pub fn next_snapshot_timestamp() -> DateTime<Utc> {
    todo!("Create the timestamp used to annotate the next configuration snapshot")
}
