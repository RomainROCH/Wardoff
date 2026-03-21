#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Scaffolding for Layer 4 remote shutdown abort handling.
pub mod remote;
/// Scaffolding for Layer 1 interactive shutdown blocking.
pub mod shutdown;
/// Scaffolding for SetThreadExecutionState-based sleep blocking.
pub mod sleep;
/// Scaffolding for Layer 3 UpdateOrchestrator reboot protection.
pub mod update;

/// Represents the global blocker mode exposed by the Wardoff core.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum BlockerMode {
    /// Actively blocks supported shutdown, reboot, and sleep paths.
    Block,
    /// Allows supported shutdown, reboot, and sleep paths to proceed.
    Allow,
}

/// Coordinates the blocker sub-systems used by the MVP runtime.
pub struct BlockerCoordinator;

/// Creates the blocker coordinator that owns the layered shutdown guards.
pub fn create_blocker_coordinator() -> BlockerCoordinator {
    todo!("Assemble the blocker coordinator from the interactive, update, remote, and sleep guards")
}

/// Returns the timestamp associated with the next blocker state transition.
pub fn next_state_change_timestamp() -> DateTime<Utc> {
    todo!("Create the timestamp recorded for the next blocker state transition")
}

impl BlockerCoordinator {
    /// Enables the layered shutdown and sleep blocking scaffolding.
    pub fn activate(&self) -> Result<(), String> {
        todo!("Enable the Layer 1, Layer 3, Layer 4, and sleep blockers")
    }

    /// Disables the layered shutdown and sleep blocking scaffolding.
    pub fn deactivate(&self) -> Result<(), String> {
        todo!("Disable the Layer 1, Layer 3, Layer 4, and sleep blockers")
    }
}
