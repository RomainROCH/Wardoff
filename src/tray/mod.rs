#![allow(dead_code)]

use crate::blocker::BlockerMode;
use crate::logger::{self, EventSource};
use log::info;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder,
};

/// Scaffolding for tray icon asset selection.
pub mod icon;

/// Controls whether the primary runtime shows or hides its tray icon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayVisibility {
    /// Creates the tray icon in its normal visible state.
    Visible,
    /// Creates the tray icon but immediately hides it.
    Hidden,
}

/// Represents the actions available from the MVP tray menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
pub struct TrayController {
    tray_icon: Option<TrayIcon>,
    visibility: TrayVisibility,
    block_item: MenuItem,
    allow_item: MenuItem,
    shutdown_item: MenuItem,
    reboot_item: MenuItem,
    sleep_item: MenuItem,
    hibernate_item: MenuItem,
    quit_item: MenuItem,
}

/// Creates the tray controller used by the background application surface.
pub fn create_tray_controller(visibility: TrayVisibility) -> Result<TrayController, String> {
    TrayController::new(visibility)
}

/// Returns the default set of tray actions exposed by the MVP menu.
pub fn default_actions() -> Vec<TrayAction> {
    vec![
        TrayAction::Block,
        TrayAction::Allow,
        TrayAction::Shutdown,
        TrayAction::Reboot,
        TrayAction::Sleep,
        TrayAction::Hibernate,
        TrayAction::Quit,
    ]
}

impl TrayController {
    /// Creates the MVP tray icon with the required right-click menu actions.
    pub fn new(visibility: TrayVisibility) -> Result<Self, String> {
        let tray_menu = Menu::new();

        let block_item = MenuItem::with_id("wardoff.block", "Block", true, None);
        let allow_item = MenuItem::with_id("wardoff.allow", "Allow", true, None);
        let shutdown_item = MenuItem::with_id("wardoff.shutdown", "Shutdown", true, None);
        let reboot_item = MenuItem::with_id("wardoff.reboot", "Reboot", true, None);
        let sleep_item = MenuItem::with_id("wardoff.sleep", "Sleep", true, None);
        let hibernate_item = MenuItem::with_id("wardoff.hibernate", "Hibernate", true, None);
        let quit_item = MenuItem::with_id("wardoff.quit", "Quit", true, None);
        let separator_one = PredefinedMenuItem::separator();
        let separator_two = PredefinedMenuItem::separator();

        tray_menu
            .append_items(&[
                &block_item,
                &allow_item,
                &separator_one,
                &shutdown_item,
                &reboot_item,
                &sleep_item,
                &hibernate_item,
                &separator_two,
                &quit_item,
            ])
            .map_err(|error| format!("Wardoff could not assemble the tray menu: {error}"))?;

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_menu_on_left_click(false)
            .with_tooltip(tooltip_for_mode(BlockerMode::Block))
            .with_icon(icon::blocked_icon().map_err(|error| {
                format!("Wardoff could not build the blocked tray icon: {error}")
            })?)
            .build()
            .map_err(|error| format!("Wardoff could not create the tray icon: {error}"))?;

        let mut controller = Self {
            tray_icon: Some(tray_icon),
            visibility,
            block_item,
            allow_item,
            shutdown_item,
            reboot_item,
            sleep_item,
            hibernate_item,
            quit_item,
        };
        controller.set_mode(BlockerMode::Block)?;

        if visibility == TrayVisibility::Hidden {
            controller.hide_tray_icon()?;
        }

        logger::log_event(
            "tray_initialized",
            EventSource::Tray,
            format!(
                "Wardoff created a {} tray icon.",
                visibility_label(visibility)
            ),
            true,
        );

        Ok(controller)
    }

    /// Applies the current blocker mode to the tray icon, tooltip, and menu state.
    pub fn set_mode(&mut self, mode: BlockerMode) -> Result<(), String> {
        self.sync_menu_state(mode);

        if self.visibility == TrayVisibility::Hidden {
            // Windows can return E_FAIL when mutating icon metadata after the tray icon is hidden.
            return Ok(());
        }

        let icon = match mode {
            BlockerMode::Block => icon::blocked_icon().map_err(|error| {
                format!("Wardoff could not render the blocked tray icon: {error}")
            })?,
            BlockerMode::Allow => icon::allowed_icon().map_err(|error| {
                format!("Wardoff could not render the allowed tray icon: {error}")
            })?,
        };

        if let Some(tray_icon) = self.tray_icon.as_ref() {
            tray_icon
                .set_icon(Some(icon))
                .map_err(|error| format!("Wardoff could not update the tray icon: {error}"))?;
            tray_icon
                .set_tooltip(Some(tooltip_for_mode(mode)))
                .map_err(|error| format!("Wardoff could not update the tray tooltip: {error}"))?;
        }

        Ok(())
    }

    /// Drains all pending tray menu activations received since the last message dispatch.
    pub fn drain_actions(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = self.action_for_menu_id(event.id()) {
                actions.push(action);
            }
        }

        actions
    }

    /// Drops the tray icon explicitly so the process can exit cleanly.
    pub fn shutdown(&mut self) {
        if self.tray_icon.take().is_some() {
            info!("Wardoff removed its tray icon during shutdown.");
            logger::log_event(
                "tray_shutdown",
                EventSource::Tray,
                "Wardoff removed its tray icon during shutdown.",
                true,
            );
        }
    }

    fn hide_tray_icon(&self) -> Result<(), String> {
        if let Some(tray_icon) = self.tray_icon.as_ref() {
            tray_icon
                .set_visible(false)
                .map_err(|error| format!("Wardoff could not hide the tray icon: {error}"))?;
        }

        logger::log_event(
            "tray_visibility_changed",
            EventSource::Tray,
            "Wardoff hid the tray icon.",
            true,
        );

        Ok(())
    }

    fn sync_menu_state(&self, mode: BlockerMode) {
        self.block_item.set_enabled(mode != BlockerMode::Block);
        self.allow_item.set_enabled(mode != BlockerMode::Allow);
    }

    fn action_for_menu_id(&self, id: &MenuId) -> Option<TrayAction> {
        if id == self.block_item.id() {
            Some(TrayAction::Block)
        } else if id == self.allow_item.id() {
            Some(TrayAction::Allow)
        } else if id == self.shutdown_item.id() {
            Some(TrayAction::Shutdown)
        } else if id == self.reboot_item.id() {
            Some(TrayAction::Reboot)
        } else if id == self.sleep_item.id() {
            Some(TrayAction::Sleep)
        } else if id == self.hibernate_item.id() {
            Some(TrayAction::Hibernate)
        } else if id == self.quit_item.id() {
            Some(TrayAction::Quit)
        } else {
            None
        }
    }
}

fn tooltip_for_mode(mode: BlockerMode) -> &'static str {
    match mode {
        BlockerMode::Block => "Wardoff - Block mode",
        BlockerMode::Allow => "Wardoff - Allow mode",
    }
}

fn visibility_label(visibility: TrayVisibility) -> &'static str {
    match visibility {
        TrayVisibility::Visible => "visible",
        TrayVisibility::Hidden => "hidden",
    }
}
