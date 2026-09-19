use std::{sync::Arc, time::Duration};

use anyhow::Context;
use basr::model::StopReason;
use tokio::sync::mpsc;
use tokio_util::task::AbortOnDropHandle;

use super::{Dbus, Notification, Notifier, Urgency};
use crate::{config::Notify, session::Completion};

const COMPLETION_CAPACITY: usize = 128;
pub const NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// One bounded worker for the daemon; queued values contain no live session handles.
pub struct Watcher {
    pub sender: mpsc::Sender<Completion>,
    task: AbortOnDropHandle<()>,
}

impl Watcher {
    /// Connects and starts the configured notifier, if enabled.
    /// # Errors
    /// Returns an error if required notifications cannot connect before the deadline.
    pub async fn connect(policy: Notify) -> anyhow::Result<Option<Self>> {
        if policy == Notify::Never {
            return Ok(None);
        }
        match tokio::time::timeout(NOTIFY_TIMEOUT, Dbus::connect()).await {
            Ok(Ok(notifier)) => Ok(Some(Self::start(Arc::new(notifier)))),
            result if policy == Notify::Always => match result {
                Ok(Err(error)) => {
                    Err(error).context("desktop notifications were required but are unavailable")
                }
                Err(error) => Err(error).context("desktop notification connection timed out"),
                Ok(Ok(_)) => unreachable!("successful connections are handled above"),
            },
            _ => {
                ::tracing::debug!("desktop notifications are unavailable");
                Ok(None)
            }
        }
    }

    pub fn start(notifier: Arc<dyn Notifier>) -> Self {
        let (sender, mut receiver) = mpsc::channel::<Completion>(COMPLETION_CAPACITY);
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            while let Some(completion) = receiver.recv().await {
                let session = completion.session.id;
                let notification = completion.into();
                match tokio::time::timeout(NOTIFY_TIMEOUT, notifier.notify(notification)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, %session, "cannot deliver completion notification")
                    }
                    Err(_) => {
                        tracing::warn!(%session, "completion notification timed out")
                    }
                }
            }
        }));
        Self { sender, task }
    }

    pub async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

impl From<Completion> for Notification {
    fn from(completion: Completion) -> Self {
        let meta = completion.session;
        let title = meta
            .title
            .unwrap_or_else(|| format!("basrd session {}", meta.id));
        let body = match completion
            .result
            .as_ref()
            .map(|response| response.stop_reason)
        {
            Ok(StopReason::EndTurn) => "The agent finished the turn.",
            Ok(StopReason::MaxTokens) => "The agent ran out of output budget.",
            Ok(StopReason::Cancelled) => "The turn was cancelled.",
            Ok(StopReason::Refusal) => "The agent declined to continue.",
            Err(_) => "The agent failed while executing the turn.",
            _ => "The turn ended.",
        };
        let urgency = match completion
            .result
            .as_ref()
            .map(|response| response.stop_reason)
        {
            Ok(StopReason::EndTurn) => Urgency::Normal,
            Err(_) => Urgency::Critical,
            _ => Urgency::Low,
        };
        Self::new(title, body).with_urgency(urgency)
    }
}
