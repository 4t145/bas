use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::{
    model::{EventKind, StopReason},
    session::{AttachOptions, Registry, Session},
};

use super::{Notification, Notifier, Urgency};

/// Starts a task that notifies the user whenever a session finishes a turn.
///
/// The task lives as long as the registry does and watches every session the
/// registry creates, so clients never have to register themselves.
#[must_use]
pub fn watch_completions(registry: Arc<Registry>, notifier: Arc<dyn Notifier>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut created = registry.subscribe();

        loop {
            match created.recv().await {
                Ok(session) => {
                    tokio::spawn(watch_session(session, Arc::clone(&notifier)));
                }
                // A watcher that missed a creation cannot recover the session,
                // so keep listening for later ones.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}

async fn watch_session(session: Arc<Session>, notifier: Arc<dyn Notifier>) {
    let mut attachment = session.attach(AttachOptions::last(0));

    loop {
        match attachment.recv().await {
            Ok(event) => {
                if let EventKind::TurnFinished { stop, .. } = event.kind {
                    notify_turn_finished(&session, stop, notifier.as_ref()).await;
                }
            }
            Err(_) => return,
        }
    }
}

async fn notify_turn_finished(session: &Session, stop: StopReason, notifier: &dyn Notifier) {
    let meta = session.meta();
    let title = meta
        .title
        .clone()
        .unwrap_or_else(|| format!("basrd session {}", meta.id));

    let notification = Notification::new(title, stop_message(stop)).with_urgency(urgency(stop));

    if let Err(error) = notifier.notify(notification).await {
        tracing::warn!(%error, session = %meta.id, "cannot deliver the completion notification");
    }
}

fn stop_message(stop: StopReason) -> &'static str {
    match stop {
        StopReason::EndTurn => "The agent finished the task.",
        StopReason::MaxTokens => "The agent ran out of output budget.",
        StopReason::Cancelled => "The task was cancelled.",
        StopReason::Refusal => "The agent declined to continue.",
        StopReason::Other => "The turn ended.",
    }
}

fn urgency(stop: StopReason) -> Urgency {
    match stop {
        StopReason::EndTurn => Urgency::Normal,
        _ => Urgency::Low,
    }
}
