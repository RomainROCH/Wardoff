mod blocker;
mod cli;
mod config;
mod logger;
mod tray;

use crate::blocker::shutdown::{run_message_loop, ShutdownBlocker};
use env_logger::{Builder, Env};
use log::{error, info, LevelFilter};
use std::error::Error;

const DEFAULT_SHUTDOWN_BLOCK_REASON: &str =
    "Wardoff is blocking shutdown while Layer 1 protection is active.";

/// Coordinates CLI, tray, and blocker scaffolding for the Wardoff binary.
pub struct Application {
    shutdown_blocker: ShutdownBlocker,
}

/// Bootstraps logging and activates Layer 1 shutdown blocking for the process.
pub fn bootstrap() -> Result<Application, Box<dyn Error>> {
    initialize_logging()?;

    let shutdown_blocker = ShutdownBlocker::new(DEFAULT_SHUTDOWN_BLOCK_REASON)?;
    info!("Wardoff Layer 1 shutdown blocking is active at startup");

    Ok(Application { shutdown_blocker })
}

/// Runs the Wardoff MVP message loop with Layer 1 protection enabled.
pub fn run(application: Application) -> Result<(), Box<dyn Error>> {
    let _shutdown_blocker = application.shutdown_blocker;
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
    builder.format_timestamp_secs().filter_level(LevelFilter::Info);
    builder.try_init()?;
    Ok(())
}
