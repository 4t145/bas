use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use basr::model::{Owner, SessionId, SessionMeta, StopReason};
use tokio::sync::Notify;

use super::{Error, Notification, Notifier, Watcher};
use crate::session::Completion;

struct PendingNotifier {
    entered: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}

#[test]
fn native_rejection_and_runtime_interruption_have_distinct_notifications() {
    use crate::session::CompletionOutcome;
    use basr::{acp, model::InterruptionReason};
    let session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    let native: Notification = Completion {
        session: session.clone(),
        outcome: CompletionOutcome::Native(Err(acp::Error::invalid_params())),
    }
    .into();
    let interrupted: Notification = Completion {
        session: session.clone(),
        outcome: CompletionOutcome::Interrupted(InterruptionReason::BackendFailure {
            message: "offline".into(),
        }),
    }
    .into();
    let cancelled: Notification = Completion {
        session,
        outcome: CompletionOutcome::Interrupted(InterruptionReason::CancelledBeforeDispatch),
    }
    .into();
    assert_eq!(native.body, "The agent rejected the prompt.");
    assert_eq!(
        interrupted.body,
        "Turn execution was interrupted by a runtime failure."
    );
    assert_eq!(cancelled.body, "The turn was cancelled before dispatch.");
    assert_eq!(native.urgency, super::Urgency::Critical);
    assert_eq!(interrupted.urgency, super::Urgency::Critical);
    assert_eq!(cancelled.urgency, super::Urgency::Low);
}

struct Guard(Arc<AtomicBool>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl Notifier for PendingNotifier {
    async fn notify(&self, _notification: Notification) -> Result<(), Error> {
        let _guard = Guard(Arc::clone(&self.dropped));
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn shutdown_cancels_a_blocked_notification_and_closes_the_queue() {
    let entered = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let watcher = Watcher::start(Arc::new(PendingNotifier {
        entered: Arc::clone(&entered),
        dropped: Arc::clone(&dropped),
    }));
    let sender = watcher.sender.clone();
    sender
        .send(Completion {
            session: SessionMeta::new(SessionId::new(), "test".into(), Owner::current()),
            outcome: crate::session::CompletionOutcome::Native(Ok(basr::acp::PromptResponse::new(
                StopReason::EndTurn,
            ))),
        })
        .await
        .expect("worker alive");
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("notification started");
    watcher.shutdown().await;
    assert!(dropped.load(Ordering::SeqCst));
    assert!(sender.is_closed());
}
