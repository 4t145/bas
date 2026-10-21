use super::*;
use crate::storage::{self, Memory, ResumeData, Storage, StoredSession};
use bas::{
    model::{RuntimeFailure, SessionSnapshot},
    service::pb::{self, bas_server::Bas},
};
use futures::{StreamExt, future::BoxFuture};

struct ControlledStore {
    inner: Memory,
    reject: AtomicBool,
    block_reads: AtomicBool,
    fail_reads: AtomicBool,
}

struct RestoredAgent;

struct ObservedAgent {
    alive: Arc<()>,
    dispatched: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl Agent for ObservedAgent {
    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }
    async fn shutdown(&mut self) {
        let _ = &self.alive;
    }
}

#[async_trait]
impl Agent for RestoredAgent {
    fn initialization(&self) -> Option<acp::InitializeResponse> {
        Some(
            serde_json::from_value(serde_json::json!({
                "protocolVersion": 1, "agentCapabilities": {}
            }))
            .expect("initialization"),
        )
    }
    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        panic!("failed runtime must never dispatch")
    }
    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }
    async fn shutdown(&mut self) {}
}

impl ControlledStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Memory::new(NonZeroUsize::new(100).expect("capacity")),
            reject: AtomicBool::new(false),
            block_reads: AtomicBool::new(false),
            fail_reads: AtomicBool::new(false),
        })
    }
}

impl Storage for ControlledStore {
    fn create(
        &self,
        meta: SessionMeta,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), storage::Error>> {
        self.inner.create(meta, resume)
    }
    fn commit<'a>(
        &'a self,
        meta: SessionMeta,
        events: &'a [Event],
        acceptance: Option<storage::PromptAcceptance>,
    ) -> BoxFuture<'a, Result<(), storage::Error>> {
        Box::pin(async move {
            if self.reject.load(Ordering::SeqCst) {
                return Err(storage::Error::InvalidCommit("controlled failure"));
            }
            self.inner.commit(meta, events, acceptance).await
        })
    }
    fn acceptance(
        &self,
        session: SessionId,
        turn: TurnId,
    ) -> BoxFuture<'_, Result<Option<storage::PromptAcceptance>, storage::Error>> {
        self.inner.acceptance(session, turn)
    }

    fn save_resume(
        &self,
        id: SessionId,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), storage::Error>> {
        self.inner.save_resume(id, resume)
    }
    fn load(&self, id: SessionId) -> BoxFuture<'_, Result<Option<StoredSession>, storage::Error>> {
        self.inner.load(id)
    }
    fn list(&self) -> BoxFuture<'_, Result<Vec<SessionMeta>, storage::Error>> {
        self.inner.list()
    }
    fn page(
        &self,
        id: SessionId,
        after: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<storage::Page, storage::Error>> {
        Box::pin(async move {
            if self.block_reads.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(storage::Error::WorkerClosed);
            }
            self.inner.page(id, after, limit).await
        })
    }
    fn window(
        &self,
        id: SessionId,
        before: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<storage::Window, storage::Error>> {
        self.inner.window(id, before, limit)
    }
    fn tail(
        &self,
        id: SessionId,
        count: usize,
    ) -> BoxFuture<'_, Result<Vec<Event>, storage::Error>> {
        self.inner.tail(id, count)
    }
    fn delete(&self, id: SessionId) -> BoxFuture<'_, Result<bool, storage::Error>> {
        self.inner.delete(id)
    }
    fn is_persistent(&self) -> bool {
        true
    }
}

fn runtime_error(error: SessionError) {
    assert!(matches!(
        error,
        SessionError::Runtime(RuntimeFailure::Persistence { .. })
    ));
    let status = tonic::Status::from(error);
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert!(status.details().is_empty());
}

#[tokio::test]
async fn every_commit_boundary_latches_failure_without_rewriting_committed_state() {
    for operation in [
        "prompt",
        "title",
        "archive",
        "completion",
        "backend",
        "shutdown",
        "restore",
    ] {
        let storage = ControlledStore::new();
        let meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
        storage.create(meta.clone(), None).await.expect("create");
        let (completions, mut completed) = mpsc::channel(4);
        let session = Session::with_storage(
            meta,
            Limits::default(),
            Some(completions),
            Some(storage.clone()),
            true,
        );
        let (updates, inbox) = mpsc::channel(16);
        session.listen(inbox);
        let alive = Arc::new(());
        let weak_agent = Arc::downgrade(&alive);
        let dispatched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session
            .ready(Box::new(ObservedAgent {
                alive,
                dispatched: dispatched.clone(),
            }))
            .await
            .expect("ready");
        let turn = if operation != "prompt" {
            Some(
                session
                    .prompt(vec!["working".into()])
                    .await
                    .expect("prompt"),
            )
        } else {
            None
        };
        let before = session.snapshot();
        let history = storage
            .page(session.id(), None, 100)
            .await
            .expect("history");
        let mut live = session
            .try_attach(AttachOptions {
                replay: 0,
                cursor: None,
            })
            .await
            .expect("attach");
        let mut replay = session
            .try_attach(AttachOptions {
                replay: 100,
                cursor: None,
            })
            .await
            .expect("replay");
        storage.block_reads.store(true, Ordering::SeqCst);
        let mut blocked = Box::pin(replay.recv());
        assert!(futures::poll!(blocked.as_mut()).is_pending());
        let mut waiting = Box::pin(live.recv());
        assert!(futures::poll!(waiting.as_mut()).is_pending());
        storage.reject.store(true, Ordering::SeqCst);
        match operation {
            "prompt" => {
                let error = session
                    .prompt(vec!["rejected".into()])
                    .await
                    .expect_err("failed commit");
                let SubmissionError::Unknown(error) = error else {
                    panic!("failed commit leaves acceptance unknown");
                };
                runtime_error(error);
            }
            "title" => runtime_error(
                session
                    .set_title("rejected")
                    .await
                    .expect_err("failed commit"),
            ),
            "archive" => runtime_error(
                session
                    .try_set_archived(true)
                    .await
                    .expect_err("failed commit"),
            ),
            "completion" => {
                session
                    .apply(Update::Turn {
                        turn: turn.expect("active turn"),
                        update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(
                            StopReason::EndTurn,
                        ))),
                    })
                    .await
            }
            "backend" => session.fail("offline".into()).await,
            "shutdown" => session.shutdown().await,
            "restore" => {
                session.suspend().await;
                runtime_error(
                    session
                        .restore_agent(
                            Box::new(RestoredAgent),
                            session.restore_gate.clone().lock_owned().await,
                        )
                        .await
                        .expect_err("failed restore commit"),
                );
            }
            _ => unreachable!(),
        }
        for error in [
            tokio::time::timeout(TIMEOUT, waiting)
                .await
                .expect("live wakes")
                .expect_err("failure"),
            tokio::time::timeout(TIMEOUT, blocked)
                .await
                .expect("history read cancelled")
                .expect_err("failure"),
        ] {
            assert!(matches!(error, AttachError::Runtime(_)));
            let status = tonic::Status::from(error);
            assert_eq!(status.code(), tonic::Code::FailedPrecondition);
            assert!(status.details().is_empty());
        }
        let failed = session.snapshot();
        assert_eq!(failed.meta, before.meta, "{operation}");
        assert_eq!(failed.runtime.id, before.runtime.id);
        assert_eq!(failed.status(), SessionStatus::Failed);
        assert_eq!(failed.active_turn(), None);
        assert!(completed.try_recv().is_err(), "no fictional ACP completion");
        assert!(updates.is_closed(), "backend updates stopped");
        assert!(
            weak_agent.upgrade().is_none(),
            "failed instance releases agent"
        );
        runtime_error(
            session
                .try_attach(AttachOptions::default())
                .await
                .err()
                .expect("new attach rejected"),
        );
        assert!(matches!(
            session
                .prompt(vec!["never dispatched".into()])
                .await
                .expect_err("terminal"),
            SubmissionError::Unknown(SessionError::Runtime(RuntimeFailure::Persistence { .. }))
        ));
        assert_eq!(
            dispatched.load(Ordering::SeqCst),
            usize::from(turn.is_some())
        );
        runtime_error(
            session
                .try_set_archived(false)
                .await
                .expect_err("even no-op is terminal"),
        );
        session.shutdown().await;
        session.fail("late failure".into()).await;
        assert_eq!(session.snapshot(), failed);
        storage.block_reads.store(false, Ordering::SeqCst);
        assert_eq!(
            storage
                .load(session.id())
                .await
                .expect("load")
                .expect("record")
                .meta,
            before.meta
        );
        assert_eq!(
            storage
                .page(session.id(), None, 100)
                .await
                .expect("history"),
            history
        );
        let mut loaded = Vec::new();
        session
            .try_visit_history(None, 100, |event, _| {
                loaded.push(event.clone());
                true
            })
            .await
            .expect("LoadHistory remains available");
        assert_eq!(loaded, history.events);
        storage.reject.store(false, Ordering::SeqCst);
        let mut recovered = Registry::with_storage(
            Arc::new(Catalog::with_defaults()),
            Arc::new(LocalHost::default()),
            Limits::default(),
            storage.clone(),
        );
        recovered
            .recover()
            .await
            .expect("recover last committed state");
        let fresh = recovered
            .get(session.id())
            .await
            .expect("recovered identity");
        assert_ne!(fresh.snapshot().runtime.id, failed.runtime.id);
        assert!(fresh.snapshot().runtime.failure.is_none());
        assert_eq!(fresh.snapshot().active_turn(), None);
        assert_eq!(fresh.snapshot().status(), SessionStatus::Idle);
        if let Some(turn) = turn {
            let events = storage
                .page(session.id(), Some(before.meta.revision), 10)
                .await
                .expect("restart event")
                .events;
            assert_eq!(events.len(), 1);
            assert!(matches!(events[0].kind, EventKind::TurnInterrupted {
                turn: interrupted,
                reason: bas::model::InterruptionReason::DaemonRestarted,
            } if interrupted == turn));
        }
        recovered.shutdown().await;
        let weak = Arc::downgrade(&session);
        drop(session);
        assert!(
            weak.upgrade().is_none(),
            "attachments must not retain session"
        );
    }
}

#[tokio::test]
async fn grpc_snapshots_and_attachments_expose_failure_and_recovery_gets_a_new_runtime() {
    let storage = ControlledStore::new();
    let registry = Arc::new(Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    ));
    let session = registry.create(request()).await.expect("create");
    let before = session.snapshot();
    let service = crate::service::Service::new(registry.clone());
    let mut stream = service
        .attach(tonic::Request::new(pb::AttachRequest {
            session: session.id().to_string(),
            replay: 0,
            cursor: None,
        }))
        .await
        .expect("attach")
        .into_inner();
    storage.reject.store(true, Ordering::SeqCst);
    runtime_error(
        session
            .set_title("cannot commit")
            .await
            .expect_err("failed commit"),
    );
    let error = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .expect("wakes")
        .expect("not EOF")
        .expect_err("failure");
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    assert!(error.details().is_empty());
    let error = service
        .attach(tonic::Request::new(pb::AttachRequest {
            session: session.id().to_string(),
            replay: 0,
            cursor: None,
        }))
        .await
        .err()
        .expect("new attach fails");
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    assert!(error.details().is_empty());
    let got = service
        .get_session(tonic::Request::new(pb::GetSessionRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("get")
        .into_inner()
        .session
        .expect("snapshot");
    let listed = service
        .list_sessions(tonic::Request::new(pb::ListSessionsRequest {
            include_archived: true,
        }))
        .await
        .expect("list")
        .into_inner()
        .sessions;
    assert_eq!(listed, vec![got.clone()]);
    let snapshot = SessionSnapshot::try_from(got).expect("snapshot conversion");
    assert_eq!(snapshot, session.snapshot());
    assert_eq!(snapshot.meta.revision, before.meta.revision);
    assert!(registry.ensure_restored(&session).await.is_err());
    registry.shutdown().await;
    storage.reject.store(false, Ordering::SeqCst);
    let mut recovered = Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    );
    recovered.recover().await.expect("recover");
    let fresh = recovered.get(session.id()).await.expect("same identity");
    assert_ne!(fresh.snapshot().runtime.id, before.runtime.id);
    assert!(fresh.snapshot().runtime.failure.is_none());
    assert_eq!(fresh.meta(), before.meta);
    recovered.remove(session.id()).await.expect("delete");
    assert!(storage.load(session.id()).await.expect("load").is_none());
}

#[tokio::test]
async fn history_read_errors_are_retryable_and_normal_drop_does_not_swallow_backlog() {
    let storage = ControlledStore::new();
    let registry = Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    );
    let session = registry.create(request()).await.expect("create");
    let before = session.snapshot();
    storage.fail_reads.store(true, Ordering::SeqCst);
    let error = session
        .try_visit_history(None, 100, |_, _| true)
        .await
        .expect_err("read error");
    assert!(matches!(error, SessionError::Storage(_)));
    assert_eq!(tonic::Status::from(error).code(), tonic::Code::Unavailable);
    assert_eq!(session.snapshot(), before);
    storage.fail_reads.store(false, Ordering::SeqCst);
    let expected = storage
        .page(session.id(), None, 100)
        .await
        .expect("retry")
        .events;
    let mut attachment = session
        .try_attach(AttachOptions {
            replay: 100,
            cursor: None,
        })
        .await
        .expect("attach");
    registry.shutdown().await;
    let weak = Arc::downgrade(&session);
    drop(session);
    assert!(weak.upgrade().is_none());
    for event in expected {
        assert_eq!(attachment.recv().await.expect("retained history"), event);
    }
    assert_eq!(attachment.recv().await, Err(AttachError::Closed));
}
