//! Login supervision for the dedicated terminal helper, never the GUI process.
//! Unix cancellation owns the helper's foreground process group, not a sandbox:
//! a program deliberately escaping with setsid is outside this boundary.

use std::{
    future::Future,
    io,
    process::{Command, ExitStatus},
};

#[derive(Debug)]
pub(super) enum Outcome {
    Exited(ExitStatus),
    Cancelled,
}

/// Run login with inherited terminal streams and cancellation of its owned group/job.
///
/// On Unix the caller must be a dedicated process-group leader, and, when stdin
/// is a terminal, its foreground group. Cancellation (including dropping this
/// future after spawning) kills that group, including this helper itself.
/// The command must not override the inherited process group or session.
///
/// # Errors
/// Returns ownership, signal setup, spawn, or wait errors. Platforms without
/// Unix groups or Windows jobs return Unsupported without spawning.
pub(super) async fn run(
    command: Command,
    cancelled: impl Future<Output = ()>,
) -> io::Result<Outcome> {
    #[cfg(unix)]
    {
        unix::run(command, cancelled).await
    }
    #[cfg(windows)]
    {
        use process_wrap::tokio::{CommandWrap, JobObject, KillOnDrop};

        let mut command = tokio::process::Command::from(command);
        inherit_streams(&mut command);
        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop).wrap(JobObject);
        let mut child = command.spawn()?;
        tokio::select! {
            biased;
            _ = cancelled => {
                child.start_kill()?;
                child.inner_mut().wait().await?;
                Ok(Outcome::Cancelled)
            }
            result = child.inner_mut().wait() => result.map(Outcome::Exited),
        }
        // Dropping the job closes its kill-on-close handle, including on errors
        // or future cancellation. Never await the wrapper's whole-job wait:
        // it uses an uncancellable blocking worker.
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (command, cancelled);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "login process-tree cancellation is unsupported on this platform",
        ))
    }
}

#[cfg(any(unix, windows))]
fn inherit_streams(command: &mut tokio::process::Command) {
    use std::process::Stdio;
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::{Pid, getpgrp, getpid, isatty, tcgetpgrp},
    };
    use std::time::Duration;
    use tokio::signal::unix::{SignalKind, signal};

    const TERMINATION_GRACE: Duration = Duration::from_millis(500);

    struct GroupGuard {
        group: Pid,
        armed: bool,
    }

    impl Drop for GroupGuard {
        fn drop(&mut self) {
            if self.armed {
                // The still-live helper is the group leader, so this ID cannot
                // be recycled. No async work or detached cleanup task is needed.
                let _ = killpg(self.group, Signal::SIGKILL);
            }
        }
    }

    pub(super) async fn run(
        command: Command,
        cancelled: impl Future<Output = ()>,
    ) -> io::Result<Outcome> {
        let group = getpid();
        if getpgrp() != group {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "terminal helper must own its process group",
            ));
        }
        let stdin = std::io::stdin();
        if isatty(&stdin).map_err(io::Error::from)?
            && tcgetpgrp(&stdin).map_err(io::Error::from)? != group
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "terminal helper must own the foreground terminal group",
            ));
        }
        // Install before spawn: these signals cancel login rather than leaving
        // it orphaned if the terminal disappears or the user presses Ctrl-C.
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut hangup = signal(SignalKind::hangup())?;
        let mut terminate = signal(SignalKind::terminate())?;
        let mut command = tokio::process::Command::from(command);
        inherit_streams(&mut command);
        // Inherit the helper's foreground group. A new child group would make
        // interactive reads stop with SIGTTIN.
        let mut child = command.spawn()?;
        let mut guard = GroupGuard { group, armed: true };
        tokio::select! {
            biased;
            _ = cancelled => {}
            _ = interrupt.recv() => {}
            _ = hangup.recv() => {}
            _ = terminate.recv() => {}
            result = child.wait() => {
                let status = result?;
                guard.armed = false;
                return Ok(Outcome::Exited(status));
            }
        }
        let _ = killpg(group, Signal::SIGTERM);
        let _ = tokio::time::timeout(TERMINATION_GRACE, child.wait()).await;
        // Kill ordinary descendants even if the direct child exited promptly.
        // This also kills us; Cancelled is normally observable only on Windows.
        drop(guard);
        Ok(Outcome::Cancelled)
    }
}

#[cfg(all(test, unix))]
mod tests;
