#![allow(dead_code)]

use std::time::Duration;
use windows::core::Result as WindowsResult;

/// Coordinates Layer 4 remote shutdown abort polling.
pub struct RemoteShutdownBlocker;

/// Returns the polling interval used by the remote shutdown abort loop.
pub fn remote_abort_interval() -> Duration {
    todo!("Return the polling interval used for repeated AbortSystemShutdown calls")
}

/// Issues the Windows call used to cancel a pending remote shutdown.
pub fn abort_remote_shutdown() -> WindowsResult<()> {
    todo!("Call AbortSystemShutdown on the local machine context to cancel pending remote shutdowns")
}

/// Starts the Layer 4 polling loop that repeatedly cancels remote shutdown requests.
pub fn start_remote_abort_loop() -> WindowsResult<()> {
    todo!("Run the remote shutdown abort polling loop while blocking remains enabled")
}
