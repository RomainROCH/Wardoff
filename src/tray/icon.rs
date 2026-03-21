#![allow(dead_code)]

use std::path::PathBuf;
use tray_icon as _;

/// Represents the visual states supported by the tray icon assets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrayIconState {
    /// Indicates that blocking is enabled.
    Blocked,
    /// Indicates that allowing shutdown and sleep is enabled.
    Allowed,
}

/// Resolves the asset path for the blocked tray icon.
pub fn blocked_icon_path() -> PathBuf {
    todo!("Resolve the asset path for the blocked tray icon variant")
}

/// Resolves the asset path for the allowed tray icon.
pub fn allowed_icon_path() -> PathBuf {
    todo!("Resolve the asset path for the allowed tray icon variant")
}
