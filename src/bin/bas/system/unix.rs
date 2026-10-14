use std::io;
use tokio::signal::unix::{SignalKind, signal};

/// # Errors
/// Returns an error when SIGINT or SIGTERM cannot be registered.
pub(super) fn shutdown_signal() -> io::Result<impl std::future::Future<Output = io::Result<()>>> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        let name =
            tokio::select! { _ = terminate.recv() => "SIGTERM", _ = interrupt.recv() => "SIGINT" };
        tracing::info!(signal = name, "shutting down");
        Ok(())
    })
}
