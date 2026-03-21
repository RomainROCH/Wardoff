use chrono::{DateTime, Utc};
use env_logger::Builder;
use log::LevelFilter;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents the source subsystem that emitted a Wardoff log event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EventSource {
    /// CLI initiated the event.
    Cli,
    /// Tray UI initiated the event.
    Tray,
    /// Interactive shutdown protection initiated the event.
    Shutdown,
    /// Windows Update reboot protection initiated the event.
    UpdateOrchestrator,
    /// Remote shutdown protection initiated the event.
    Remote,
    /// Sleep or hibernate protection initiated the event.
    Sleep,
}

/// Describes the schema of a single rotating JSON lines record.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogRecord;

/// Creates the logger builder used for Wardoff startup scaffolding.
pub fn logger_builder() -> Builder {
    todo!("Create the env_logger builder used to initialize Wardoff logging")
}

/// Returns the default filter level for Wardoff operational logs.
pub fn default_level() -> LevelFilter {
    todo!("Return the default log level used by the Wardoff runtime")
}

/// Returns the filesystem path used for the rotating JSON lines log file.
pub fn default_log_path() -> PathBuf {
    todo!("Resolve the default filesystem path for the Wardoff JSON lines log file")
}

/// Returns the timestamp at which the next log rotation check should happen.
pub fn next_rotation_check() -> DateTime<Utc> {
    todo!("Compute the timestamp for the next rotating log file maintenance pass")
}
