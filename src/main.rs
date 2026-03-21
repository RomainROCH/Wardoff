mod blocker;
mod cli;
mod config;
mod logger;
mod tray;

use crate::blocker::remote::RemoteShutdownBlocker;
use crate::blocker::shutdown::{run_message_loop, ShutdownBlocker};
use crate::blocker::sleep::SleepBlocker;
use crate::blocker::update::UpdateRebootBlocker;
use env_logger::{Builder, Env};
use log::{error, info, LevelFilter};
use std::error::Error;

const DEFAULT_SHUTDOWN_BLOCK_REASON: &str =
    "Wardoff is blocking shutdown while Layer 1 protection is active.";

/// Coordinates CLI, tray, and blocker scaffolding for the Wardoff binary.
pub struct Application {
    shutdown_blocker: ShutdownBlocker,
    update_reboot_blocker: UpdateRebootBlocker,
    remote_shutdown_blocker: RemoteShutdownBlocker,
    sleep_blocker: SleepBlocker,
}

/// Bootstraps logging and activates Layers 1, 3, 4, and sleep blocking for the process.
pub fn bootstrap() -> Result<Application, Box<dyn Error>> {
    initialize_logging()?;

    let shutdown_blocker = ShutdownBlocker::new(DEFAULT_SHUTDOWN_BLOCK_REASON)?;
    info!("Wardoff Layer 1 shutdown blocking is active at startup");

    let update_reboot_blocker = UpdateRebootBlocker::start_blocking();
    let remote_shutdown_blocker = RemoteShutdownBlocker::start_blocking();
    let sleep_blocker = SleepBlocker::start_blocking();

    Ok(Application {
        shutdown_blocker,
        update_reboot_blocker,
        remote_shutdown_blocker,
        sleep_blocker,
    })
}

/// Runs the Wardoff MVP message loop with Layers 1, 3, 4, and sleep blocking enabled.
pub fn run(application: Application) -> Result<(), Box<dyn Error>> {
    let Application {
        shutdown_blocker,
        update_reboot_blocker,
        remote_shutdown_blocker,
        sleep_blocker,
    } = application;
    let _shutdown_blocker = shutdown_blocker;
    let _update_reboot_blocker = update_reboot_blocker;
    let _remote_shutdown_blocker = remote_shutdown_blocker;
    let _sleep_blocker = sleep_blocker;
    run_message_loop()?;
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
