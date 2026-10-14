//! Process signals and optional Linux systemd integration.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(unix)]
mod unix;

use std::{io, time::Duration};
use tokio_util::task::AbortOnDropHandle;

pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub fn ready() {
    #[cfg(target_os = "linux")]
    linux::ready();
}

pub fn stopping() {
    #[cfg(target_os = "linux")]
    linux::stopping();
}

pub fn spawn_watchdog() -> Option<AbortOnDropHandle<()>> {
    #[cfg(target_os = "linux")]
    {
        linux::spawn_watchdog()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Installs signal listeners before reporting readiness.
///
/// # Errors
/// Returns an error when signal handlers cannot be installed.
pub fn shutdown_signal() -> io::Result<impl std::future::Future<Output = io::Result<()>>> {
    #[cfg(unix)]
    {
        unix::shutdown_signal()
    }
    #[cfg(not(unix))]
    {
        Ok(tokio::signal::ctrl_c())
    }
}
