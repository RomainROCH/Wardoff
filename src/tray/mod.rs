use log::{error, info, warn};
use tray_icon as _;

/// Scaffolding for tray icon asset selection.
pub mod icon;

/// Represents the actions available from the MVP tray menu.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrayAction {
    /// Switches the application into blocking mode.
    Block,
    /// Switches the application into allow mode.
    Allow,
    /// Starts a standard shutdown flow from the tray.
    Shutdown,
    /// Starts a standard reboot flow from the tray.
    Reboot,
    /// Requests sleep from the tray menu.
    Sleep,
    /// Requests hibernate from the tray menu.
    Hibernate,
    /// Terminates the Wardoff process.
    Quit,
}

/// Coordinates tray creation and menu event dispatch.
pub struct TrayController;

/// Creates the tray controller used by the background application surface.
pub fn create_tray_controller() -> TrayController {
    todo!("Assemble the tray controller with the MVP block, allow, power, and quit commands")
}

/// Returns the default set of tray actions exposed by the MVP menu.
pub fn default_actions() -> Vec<TrayAction> {
    todo!("Return the ordered list of tray actions exposed by the MVP tray menu")
}
