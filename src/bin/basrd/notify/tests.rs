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
            result: Ok(basr::acp::PromptResponse::new(StopReason::EndTurn)),
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
