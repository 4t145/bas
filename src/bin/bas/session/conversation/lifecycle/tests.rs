use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const IDLE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Counts {
    released: AtomicUsize,
    closed: AtomicUsize,
}

struct Reclaimable(Arc<Counts>, Option<mpsc::Sender<Update>>);

#[async_trait::async_trait]
impl Agent for Reclaimable {
    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        Ok(())
    }

    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }

    fn try_idle_reclaim(&mut self, _: Duration) -> bool {
        true
    }

    async fn drain_updates(&mut self) {
        if let Some(updates) = &self.1 {
            updates
                .send(Update::Acp(acp::SessionUpdate::AgentMessageChunk(
                    acp::ContentChunk::new("queued-before-seal".into()),
                )))
                .await
                .expect("queued update");
            let (ack, receipt) = tokio::sync::oneshot::channel();
            updates.send(Update::Barrier(ack)).await.expect("barrier");
            receipt.await.expect("queued events committed");
        }
    }

    async fn release(&mut self) {
        self.0.released.fetch_add(1, Ordering::SeqCst);
    }

    async fn shutdown(&mut self) {
        self.0.closed.fetch_add(1, Ordering::SeqCst);
    }
}

async fn session() -> (Arc<Session>, Arc<Counts>) {
    let limits = Limits::default();
    let storage: Arc<dyn Storage> = Arc::new(storage::Memory::new(limits.history_capacity));
    let mut meta = SessionMeta::new(SessionId::new(), "acp".into(), bas::model::Owner::current());
    meta.status = SessionStatus::Idle;
    storage
        .create(meta.clone(), Some(storage::ResumeData::acp("original")))
        .await
        .expect("create");
    let session = Session::with_storage(meta, limits, None, Some(storage), true);
    session.remember_backend_id("original".into());
    let counts = Arc::new(Counts::default());
    session.lock().agent = Some(Box::new(Reclaimable(Arc::clone(&counts), None)));
    expire(&session);
    (session, counts)
}

fn expire(session: &Session) {
    session.lock().last_backend_use = tokio::time::Instant::now() - IDLE - Duration::from_secs(1);
}

async fn reclaim(session: &Arc<Session>) -> bool {
    session
        .reclaim_idle(
            IDLE,
            Arc::new(agent::permissions::Permissions::default()),
            Arc::new(agent::elicitation::Elicitations::default()),
        )
        .await
}

#[tokio::test]
async fn reclamation_preserves_history_identity_and_live_subscription() {
    let (session, counts) = session().await;
    session.set_title("before").await.expect("title");
    let before = session.snapshot();
    let mut attachment = session
        .try_attach(AttachOptions {
            replay: 0,
            cursor: Some(before.meta.revision),
        })
        .await
        .expect("attach");
    let generation = session.lock().backend_generation;
    assert!(reclaim(&session).await);
    assert!(!session.is_restored());
    assert_eq!(session.snapshot(), before);
    assert_eq!(
        session.backend_id.get().map(String::as_str),
        Some("original")
    );
    assert_eq!(counts.released.load(Ordering::SeqCst), 1);
    assert_eq!(counts.closed.load(Ordering::SeqCst), 0);
    assert!(!reclaim(&session).await);

    let gate = Arc::clone(&session.restore_gate).lock_owned().await;
    session
        .restore_agent(Box::new(Reclaimable(counts, None)), gate)
        .await
        .expect("restore");
    session
        .apply_from(generation, Update::Failed("old runtime".into()))
        .await;
    assert_eq!(session.meta().status, SessionStatus::Idle);
    session
        .set_title("after")
        .await
        .expect("live after restoration");
    let event = tokio::time::timeout(Duration::from_secs(1), attachment.recv())
        .await
        .expect("subscriber survives")
        .expect("event");
    assert_eq!(event.cursor, before.meta.revision.next());
    assert_eq!(
        event.kind,
        EventKind::TitleChanged {
            title: "after".into()
        }
    );
    let mut history = Vec::new();
    session
        .try_visit_history(None, 10, |event, _| {
            history.push(event.clone());
            true
        })
        .await
        .expect("history");
    assert_eq!(history.len(), 2);
}

#[tokio::test]
async fn backend_lease_pins_idle_runtime_and_restarts_idle_clock() {
    let (session, _) = session().await;
    let lease = session.use_backend();
    expire(&session);
    assert!(!reclaim(&session).await);
    drop(lease);
    assert!(
        !reclaim(&session).await,
        "operation completion resets timeout"
    );
    expire(&session);
    assert!(reclaim(&session).await);
}

#[tokio::test]
async fn active_turn_reload_and_restore_gate_prevent_reclamation() {
    let (session, _) = session().await;
    session.lock().active = Some(TurnId::new());
    assert!(!reclaim(&session).await);
    session.lock().active = None;
    session.lock().reloading = true;
    assert!(!reclaim(&session).await);
    session.lock().reloading = false;
    let gate = session.restore_gate.lock().await;
    assert!(!reclaim(&session).await);
    drop(gate);
    assert!(reclaim(&session).await);
}

#[tokio::test]
async fn retirement_drains_queued_history_before_discarding_the_listener() {
    let (session, counts) = session().await;
    let (updates, inbox) = mpsc::channel(4);
    session.listen(inbox);
    session.lock().agent = Some(Box::new(Reclaimable(counts, Some(updates))));
    tokio::time::timeout(Duration::from_secs(1), reclaim(&session))
        .await
        .expect("drain must not deadlock")
        .then_some(())
        .expect("reclaimed");
    let mut history = Vec::new();
    session
        .try_visit_history(None, 10, |event, _| {
            history.push(event.clone());
            true
        })
        .await
        .expect("history");
    assert_eq!(history.len(), 1);
    assert!(
        serde_json::to_string(&history)
            .expect("JSON")
            .contains("queued-before-seal")
    );
}

#[tokio::test]
async fn unknown_subagent_activity_prevents_reclamation() {
    let (session, _) = session().await;
    session
        .lock()
        .meta
        .subagents
        .apply(None, &acp::SubagentUpdate::new("child"))
        .expect("unknown activity child");
    assert!(!reclaim(&session).await);
}
