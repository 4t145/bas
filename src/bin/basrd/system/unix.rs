//! Unix signal handling.

use std::io;

use tokio::signal::unix::{SignalKind, signal};

/// Resolves on `SIGINT` or `SIGTERM`.
///
/// systemd sends `SIGTERM` on `systemctl stop`; `SIGINT` covers an interactive
/// `Ctrl-C` during development.
///
/// # Errors
///
/// Returns an error when the signal handlers cannot be installed.
pub(super) async fn shutdown_signal() -> io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;

    let name = tokio::select! {
        _ = terminate.recv() => "SIGTERM",
        _ = interrupt.recv() => "SIGINT",
    };

    tracing::info!(signal = name, "shutting down");
    Ok(())
}
