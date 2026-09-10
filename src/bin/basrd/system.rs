//! systemd integration and process lifetime.
//!
//! The daemon is designed to run as a systemd service, including in constrained
//! environments such as an installer. See `sd_notify(3)` for the notification
//! protocol and `systemd.service(5)` for `Type=notify` and `WatchdogSec=`.

#[cfg(unix)]
mod unix;

use std::{io, time::Duration};

use sd_notify::NotifyState;
use tokio::task::JoinHandle;

/// Fraction of the watchdog interval at which the daemon pings the service
/// manager. Pinging twice per interval tolerates one missed tick.
const WATCHDOG_PING_DIVISOR: u32 = 2;

/// How long in-flight work may delay shutdown before the daemon exits anyway.
///
/// Attached clients hold long-lived streams, so draining can never be relied on
/// to finish. `systemd` would `SIGKILL` after `TimeoutStopSec` regardless.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Tells the service manager the daemon is accepting connections.
pub fn ready() {
    notify(&[NotifyState::Ready]);
}

/// Tells the service manager the daemon is shutting down.
pub fn stopping() {
    notify(&[NotifyState::Stopping]);
}

/// Starts watchdog pings when the unit sets `WatchdogSec=`.
#[must_use]
pub fn spawn_watchdog() -> Option<JoinHandle<()>> {
    let interval = sd_notify::watchdog_enabled()?;
    let ping = interval / WATCHDOG_PING_DIVISOR;

    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(ping);
        loop {
            ticker.tick().await;
            notify(&[NotifyState::Watchdog]);
        }
    }))
}

/// Resolves when the process is asked to stop.
///
/// # Errors
///
/// Returns an error when the signal handlers cannot be installed.
pub async fn shutdown_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::shutdown_signal().await
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

fn notify(states: &[NotifyState]) {
    if let Err(error) = sd_notify::notify(states) {
        // Not running under systemd is normal; a failed notification is not
        // worth taking the daemon down for.
        tracing::debug!(%error, "cannot notify the service manager");
    }
}
