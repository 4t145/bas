//! One stdin lease spans the prompt and its reader's teardown. Dropping a prompt
//! synchronously signals cancellation; the supervisor joins the dedicated OS
//! thread before releasing that lease. Neither reads nor joins block Tokio.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::{Mutex, OwnedMutexGuard, oneshot, watch};

#[cfg(unix)]
#[path = "terminal/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "terminal/windows.rs"]
mod platform;

const READER_CHECK_INTERVAL: Duration = Duration::from_millis(10);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);

static STDIN: LazyLock<Arc<Gate>> = LazyLock::new(|| Arc::new(Gate::default()));

struct Gate {
    lease: Arc<Mutex<()>>,
    failure: watch::Sender<Option<String>>,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            lease: Arc::new(Mutex::new(())),
            failure: watch::channel(None).0,
        }
    }
}

impl Gate {
    /// # Errors
    /// Rejects stdin reuse after a reader could not be cancelled.
    async fn acquire(self: &Arc<Self>) -> io::Result<Session> {
        let lease = self.lock(self.lease.clone()).await?;
        Ok(Session {
            gate: self.clone(),
            lease: Arc::new(lease),
            reading: Arc::new(Mutex::new(())),
            #[cfg(any(windows, test))]
            printer: reedline::ExternalPrinter::default(),
        })
    }

    /// # Errors
    /// Wakes waiting sessions/reads with the cancellation fault instead of hanging.
    async fn lock(&self, mutex: Arc<Mutex<()>>) -> io::Result<OwnedMutexGuard<()>> {
        let mut failure = self.failure.subscribe();
        tokio::select! {
            biased;
            failed = failure.wait_for(Option::is_some) => {
                Err(io::Error::other(
                    failed.expect("gate owns the sender")
                        .as_ref().expect("waited for a failure").clone(),
                ))
            }
            lease = mutex.lock_owned() => {
                self.check()?;
                Ok(lease)
            }
        }
    }

    /// # Errors
    /// Returns the first cancellation failure, if any.
    fn check(&self) -> io::Result<()> {
        match self.failure.borrow().as_ref() {
            Some(error) => Err(io::Error::other(error.clone())),
            None => Ok(()),
        }
    }

    fn fail(&self, error: io::Error) {
        self.failure.send_if_modified(|failure| {
            if failure.is_some() {
                return false;
            }
            *failure = Some(format!(
                "Terminal input failed: {error}; stdin cannot be reused"
            ));
            true
        });
    }
}

pub(super) struct Session {
    gate: Arc<Gate>,
    lease: Arc<OwnedMutexGuard<()>>,
    reading: Arc<Mutex<()>>,
    #[cfg(any(windows, test))]
    printer: reedline::ExternalPrinter<String>,
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        // Must not depend on the supervisor being scheduled: the runtime may
        // shut down immediately after the command drops its input future.
        self.0.store(true, Ordering::Release);
    }
}

impl Session {
    /// # Errors
    /// Rejects stdin reuse if a previous reader failed to stop.
    pub(super) async fn acquire() -> io::Result<Self> {
        STDIN.acquire().await
    }

    /// # Errors
    /// Reports cancellation, EOF, invalid input, or thread/terminal failures.
    pub(super) async fn read_line(&self) -> io::Result<String> {
        #[cfg(unix)]
        {
            self.read_with(|cancelled| platform::line(&cancelled), CANCEL_TIMEOUT)
                .await
        }
        #[cfg(windows)]
        {
            let gate = self.gate.clone();
            let printer = self.printer.clone();
            self.read_with(
                move |cancelled| platform::line(cancelled, gate, printer),
                CANCEL_TIMEOUT,
            )
            .await
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Cancellable terminal input is unsupported on this platform",
            ))
        }
    }

    /// # Errors
    /// Reports worker startup, read, and worker panic failures.
    async fn read_with(
        &self,
        read: impl FnOnce(Arc<AtomicBool>) -> io::Result<String> + Send + 'static,
        timeout: Duration,
    ) -> io::Result<String> {
        // Even a caller retaining its Session after dropping one read future
        // cannot start another read until the supervisor has joined the old one.
        let reading = Arc::new(self.gate.lock(self.reading.clone()).await?);
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = CancelOnDrop(cancelled.clone());
        let worker_cancelled = cancelled.clone();
        // The worker also owns both leases, so runtime shutdown cannot release
        // stdin while a detached supervisor's worker is still restoring modes.
        let worker_lease = self.lease.clone();
        let worker_reading = reading.clone();
        let (result_send, mut result_receive) = oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("bast-elicitation-input".into())
            .spawn(move || {
                let _lease = worker_lease;
                let _reading = worker_reading;
                let _ = result_send.send(read(worker_cancelled));
            })?;
        let (mut reply, receive) = oneshot::channel();
        let gate = self.gate.clone();
        let lease = self.lease.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let _reading = reading;
            let result = tokio::select! {
                biased;
                _ = reply.closed() => {
                    if let Err(error) = stop_reader(&worker, timeout).await {
                        // A broken/non-cancellable driver must not hang the executor
                        // or allow another consumer. Wake lease waiters with an error,
                        // but retain this worker and lease until it really exits.
                        gate.fail(error);
                    }
                    result_receive.await
                }
                result = &mut result_receive => result,
            };
            while !worker.is_finished() {
                tokio::time::sleep(READER_CHECK_INTERVAL).await;
            }
            let joined = worker.join();
            let result = match (joined, result) {
                (Ok(()), Ok(result)) => result,
                _ => Err(io::Error::other("Terminal reader thread failed")),
            };
            let _ = reply.send(result);
        });
        let result = receive
            .await
            .map_err(|_| io::Error::other("Terminal reader supervisor closed"))?;
        self.finish().await?;
        result
    }

    /// Wait for a dropped input future to stop and restore modes before printing
    /// terminal status, returning an error, or displaying another field.
    ///
    /// # Errors
    /// Reports a faulted reader that cannot safely hand stdin back.
    pub(super) async fn finish(&self) -> io::Result<()> {
        let _reading = self.gate.lock(self.reading.clone()).await?;
        #[cfg(any(windows, test))]
        self.flush_progress(&mut |line| println!("{line}"));
        Ok(())
    }

    /// Cancellation-safe: a message is emitted/enqueued only on `Ready`.
    ///
    /// # Errors
    /// Reports a faulted reader. A full printer never blocks the Tokio thread.
    pub(super) async fn progress(&self, message: &str) -> io::Result<()> {
        #[cfg(any(windows, test))]
        {
            self.progress_with(message, |line| println!("{line}")).await
        }
        #[cfg(not(any(windows, test)))]
        {
            crate::safe::safe_println!("{message}");
            Ok(())
        }
    }

    /// # Errors
    /// Reports a faulted input gate while waiting for printer capacity.
    #[cfg(any(windows, test))]
    async fn progress_with(&self, message: &str, mut emit: impl FnMut(String)) -> io::Result<()> {
        // Reedline splits queued strings with `lines()`. Include println's final
        // newline so empty messages and trailing blank lines remain visible.
        let mut message = format!("{}\n", crate::safe::Safe(message));
        loop {
            self.gate.check()?;
            if let Ok(_reading) = self.reading.try_lock() {
                self.flush_progress(&mut emit);
                message.pop(); // The direct printer adds the framing newline.
                emit(message);
                return Ok(());
            }
            match self.printer.sender().try_send(message) {
                Ok(()) => return Ok(()),
                Err(error) => message = error.into_inner(),
            }
            // The caller selects this future alongside server events, so even a
            // full queue cannot hide cancellation or a final readiness check.
            tokio::time::sleep(READER_CHECK_INTERVAL).await;
        }
    }

    #[cfg(any(windows, test))]
    fn flush_progress(&self, emit: &mut impl FnMut(String)) {
        // Only called with the read lease: never compete with the live editor's
        // cloned receiver. Queued messages outlive the editor that was cancelled.
        while let Some(mut line) = self.printer.get_line() {
            line.pop(); // Every queued message includes the final output newline.
            emit(line);
        }
    }
}

/// # Errors
/// Reports a reader that does not finish after being signalled.
async fn stop_reader(worker: &JoinHandle<()>, timeout: Duration) -> io::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while !worker.is_finished() {
        if tokio::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "The terminal reader did not finish cancelling its read",
            ));
        }
        tokio::time::sleep(READER_CHECK_INTERVAL).await;
    }
    Ok(())
}

/// # Errors
/// Reports that the owner has dropped the read future.
fn check_cancelled(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Input request cancelled",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
