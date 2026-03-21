mod blocker;
mod cli;
mod config;
mod logger;
mod tray;

use crate::blocker::{
    create_blocker_coordinator, execute_power_action, BlockerCoordinator, BlockerMode, PowerAction,
};
use crate::tray::{create_tray_controller, TrayAction, TrayController};
use env_logger::{Builder, Env};
use log::{error, info, LevelFilter};
use std::error::Error;
use std::io;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, WaitMessage, MSG, PM_REMOVE, WM_QUIT,
};

/// Coordinates CLI, tray, and blocker scaffolding for the Wardoff binary.
pub struct Application {
    blocker_coordinator: BlockerCoordinator,
    tray_controller: TrayController,
}

/// Bootstraps logging, the central state manager, and the tray surface.
pub fn bootstrap() -> Result<Application, Box<dyn Error>> {
    initialize_logging()?;

    let blocker_coordinator = create_blocker_coordinator().map_err(other_error)?;
    let mut tray_controller =
        create_tray_controller().map_err(|message| other_error(message.to_string()))?;
    tray_controller
        .set_mode(blocker_coordinator.mode())
        .map_err(other_error)?;

    info!("Wardoff started in Block mode and initialized its tray icon.");

    Ok(Application {
        blocker_coordinator,
        tray_controller,
    })
}

/// Runs the shared Win32 event loop for the hidden Layer 1 windows and the tray icon.
pub fn run(mut application: Application) -> Result<(), Box<dyn Error>> {
    let mut message = MSG::default();

    'message_loop: loop {
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                break 'message_loop;
            }

            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            if application.process_tray_actions() {
                break 'message_loop;
            }
        }

        if application.process_tray_actions() {
            break;
        }

        unsafe { WaitMessage()? };
    }

    application.shutdown()?;
    Ok(())
}

fn main() {
    if let Err(error) = bootstrap().and_then(run) {
        error!("Wardoff failed to start: {error}");
        eprintln!("wardoff failed: {error}");
        std::process::exit(1);
    }
}

fn initialize_logging() -> Result<(), Box<dyn Error>> {
    let mut builder = Builder::from_env(Env::default().default_filter_or("info"));
    builder
        .format_timestamp_secs()
        .filter_level(LevelFilter::Info);
    builder.try_init()?;
    Ok(())
}

impl Application {
    fn process_tray_actions(&mut self) -> bool {
        for action in self.tray_controller.drain_actions() {
            match self.handle_tray_action(action) {
                Ok(true) => return true,
                Ok(false) => {}
                Err(message) => error!(
                    "Wardoff could not process tray action {:?}: {message}",
                    action
                ),
            }
        }

        false
    }

    fn handle_tray_action(&mut self, action: TrayAction) -> Result<bool, String> {
        match action {
            TrayAction::Block => {
                self.set_mode(BlockerMode::Block)?;
                Ok(false)
            }
            TrayAction::Allow => {
                self.set_mode(BlockerMode::Allow)?;
                Ok(false)
            }
            TrayAction::Shutdown => {
                self.run_power_action(PowerAction::Shutdown)?;
                Ok(false)
            }
            TrayAction::Reboot => {
                self.run_power_action(PowerAction::Reboot)?;
                Ok(false)
            }
            TrayAction::Sleep => {
                self.run_power_action(PowerAction::Sleep)?;
                Ok(false)
            }
            TrayAction::Hibernate => {
                self.run_power_action(PowerAction::Hibernate)?;
                Ok(false)
            }
            TrayAction::Quit => Ok(true),
        }
    }

    fn run_power_action(&mut self, action: PowerAction) -> Result<(), String> {
        let previous_mode = self.blocker_coordinator.mode();
        self.set_mode(BlockerMode::Allow)?;

        match execute_power_action(action) {
            Ok(()) => {
                info!(
                    "Wardoff switched to Allow mode and issued the requested {:?} power action.",
                    action
                );
                if matches!(action, PowerAction::Sleep | PowerAction::Hibernate)
                    && previous_mode == BlockerMode::Block
                {
                    info!(
                        "Wardoff will remain in Allow mode after wake. Use the tray menu to return to Block mode when you need shutdown protection again."
                    );
                }
                Ok(())
            }
            Err(message) => {
                if previous_mode == BlockerMode::Block {
                    return match self.set_mode(BlockerMode::Block) {
                        Ok(()) => Err(format!(
                            "{message} Wardoff restored Block mode after the failed {:?} request.",
                            action
                        )),
                        Err(restore_error) => Err(format!(
                            "{message} Wardoff could not restore Block mode after the failed {:?} request and remains in {} mode: {restore_error}",
                            action,
                            mode_label(self.blocker_coordinator.mode())
                        )),
                    };
                }

                Err(format!(
                    "{message} Wardoff remains in {} mode.",
                    mode_label(self.blocker_coordinator.mode())
                ))
            }
        }
    }

    fn set_mode(&mut self, mode: BlockerMode) -> Result<(), String> {
        let previous_mode = self.blocker_coordinator.mode();

        if previous_mode == mode {
            return self.tray_controller.set_mode(previous_mode);
        }

        self.blocker_coordinator.set_mode(mode)?;

        if let Err(tray_error) = self
            .tray_controller
            .set_mode(self.blocker_coordinator.mode())
        {
            let rollback_result = self.blocker_coordinator.set_mode(previous_mode);
            let final_mode = self.blocker_coordinator.mode();
            let tray_resync_result = self.tray_controller.set_mode(final_mode);

            let mut error_message = format!(
                "Wardoff could not update the tray UI after switching to {} mode: {tray_error}",
                mode_label(mode)
            );

            match rollback_result {
                Ok(_) => error_message.push_str(&format!(
                    " Wardoff rolled the blocker state back to {} mode.",
                    mode_label(previous_mode)
                )),
                Err(rollback_error) => error_message.push_str(&format!(
                    " Wardoff could not roll the blocker state back to {} mode and is currently in {} mode: {rollback_error}",
                    mode_label(previous_mode),
                    mode_label(final_mode)
                )),
            }

            if let Err(tray_resync_error) = tray_resync_result {
                error_message.push_str(&format!(
                    " Wardoff also could not resync the tray UI to {} mode: {tray_resync_error}",
                    mode_label(final_mode)
                ));
            }

            return Err(error_message);
        }

        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), Box<dyn Error>> {
        let blocker_shutdown = self.blocker_coordinator.shutdown().map_err(other_error);
        self.tray_controller.shutdown();
        blocker_shutdown.map_err(|error| Box::new(error) as Box<dyn Error>)
    }
}

fn other_error(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message)
}

fn mode_label(mode: BlockerMode) -> &'static str {
    match mode {
        BlockerMode::Block => "Block",
        BlockerMode::Allow => "Allow",
    }
}
