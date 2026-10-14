//! Owns one noninteractive installation command and its process group or job.
//! Descendants that deliberately escape a Unix group are outside this boundary.

use std::{future::Future, io, process::ExitStatus};
use tokio::process::Command;

#[derive(Debug)]
pub(super) enum Outcome {
    Exited(ExitStatus),
    Cancelled,
}

/// Stop the contained group/job on every exit path, preserving the root's exit status.
///
/// Normal completion and cooperative cancellation reap the direct child before
/// returning. Dropping the future still stops the group/job synchronously; Tokio
/// owns the abandoned child's reaping. Descendant reaping belongs to the OS.
///
/// # Errors
/// Returns containment setup, spawn, signal or wait failures.
pub(super) async fn run(
    command: Command,
    cancelled: impl Future<Output = ()>,
) -> io::Result<Outcome> {
    tokio::pin!(cancelled);
    tokio::select! {
        biased;
        () = &mut cancelled => return Ok(Outcome::Cancelled),
        () = std::future::ready(()) => {}
    }
    #[cfg(unix)]
    {
        unix::run(command, cancelled).await
    }
    #[cfg(windows)]
    {
        use process_wrap::tokio::{CommandWrap, JobObject, KillOnDrop};

        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop).wrap(JobObject);
        let mut child = command.spawn()?;
        let outcome = tokio::select! {
            biased;
            () = &mut cancelled => {
                child.start_kill()?;
                child.inner_mut().wait().await?;
                Outcome::Cancelled
            }
            status = child.inner_mut().wait() => Outcome::Exited(status?),
        };
        // Job ownership survives root exit; closing the kill-on-close handle
        // terminates residual descendants without a blocking whole-job waiter.
        drop(child);
        Ok(outcome)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = command;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "installation process containment is unsupported on this platform",
        ))
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use nix::{
        errno::Errno,
        sys::signal::{Signal as UnixSignal, killpg},
        unistd::Pid,
    };
    use tokio::signal::unix::{Signal, SignalKind, signal};

    struct Group {
        child: tokio::process::Child,
        id: Option<Pid>,
    }

    impl Group {
        fn terminate(&mut self) -> io::Result<()> {
            if let Some(id) = self.id {
                match killpg(id, UnixSignal::SIGKILL) {
                    Ok(()) | Err(Errno::ESRCH) => self.id = None,
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        }

        async fn exited(&mut self, changed: &mut Signal) -> io::Result<()> {
            let id = self.id.expect("group remains owned until termination");
            loop {
                match exited_without_reaping(id) {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(Errno::EINTR) => continue,
                    Err(Errno::ECHILD) => {
                        // An external reaper violated child ownership. Do not
                        // signal a numeric group ID that might now be reused.
                        self.id = None;
                        return Err(Errno::ECHILD.into());
                    }
                    Err(error) => return Err(error.into()),
                }
                changed.recv().await.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "SIGCHLD stream closed")
                })?;
            }
        }
    }

    impl Drop for Group {
        fn drop(&mut self) {
            if let Err(error) = self.terminate() {
                tracing::warn!(%error, "could not stop installation process group");
            }
        }
    }

    pub(super) async fn run(
        mut command: Command,
        cancelled: impl Future<Output = ()>,
    ) -> io::Result<Outcome> {
        if !cfg!(any(
            target_os = "android",
            target_os = "freebsd",
            target_os = "haiku",
            target_os = "macos",
            all(target_os = "linux", not(target_env = "uclibc"))
        )) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "non-reaping process observation is unsupported on this platform",
            ));
        }
        let mut changed = signal(SignalKind::child())?;
        // Only Group may signal the numeric group ID. The root is not reaped
        // until the group has been stopped and this guard disarmed.
        command.process_group(0).kill_on_drop(false);
        let child = command.spawn()?;
        let id = Pid::from_raw(child.id().expect("new child has a process ID") as i32);
        let mut group = Group {
            child,
            id: Some(id),
        };
        let completed = tokio::select! {
            biased;
            () = cancelled => false,
            result = group.exited(&mut changed) => {
                result?;
                true
            }
        };
        group.terminate()?;
        let status = group.child.wait().await?;
        Ok(if completed {
            Outcome::Exited(status)
        } else {
            Outcome::Cancelled
        })
    }

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "haiku",
        all(target_os = "linux", not(target_env = "uclibc"))
    ))]
    fn exited_without_reaping(id: Pid) -> Result<bool, Errno> {
        use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
        waitid(
            Id::Pid(id),
            WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
        )
        .map(|status| status != WaitStatus::StillAlive)
    }

    #[cfg(target_os = "macos")]
    fn exited_without_reaping(id: Pid) -> Result<bool, Errno> {
        use nix::libc;
        // nix does not expose waitid on macOS. WNOWAIT keeps the root PID
        // reserved while we terminate its group, just as on the other hosts.
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                id.as_raw() as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        Errno::result(result)?;
        Ok(unsafe { info.assume_init() }.si_pid != 0)
    }

    #[cfg(not(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "haiku",
        target_os = "macos",
        all(target_os = "linux", not(target_env = "uclibc"))
    )))]
    fn exited_without_reaping(_id: Pid) -> Result<bool, Errno> {
        Err(Errno::ENOSYS)
    }
}

#[cfg(all(test, any(unix, windows)))]
mod tests;
