use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use basr::{
    acp,
    model::{
        ContentBlock, Cursor, Event, EventKind, Owner, SessionId, SessionMeta, SessionStatus,
        TurnId,
    },
};
use futures::future::BoxFuture;
use tokio::sync::{Notify, mpsc};

use super::{
    AttachOptions, Catalog, CreateError, Limits, LocalHost, PendingAgent, Registry, Session,
    request,
};
use crate::{
    agent::{self, Agent, AgentSpec, Driver, Kind, LaunchRequest, Update},
    storage::{self, LocalDb, Memory, ResumeData, Storage, StoredSession},
};

/// Holds a matching commit until the test allows its durable write to complete.
struct PausedCommit {
    inner: Memory,
    entered: Notify,
    resume: Notify,
    pause_on: fn(&Event) -> bool,
    create_gate: Option<CreateGate>,
}

#[derive(Default)]
struct CreateGate {
    entered: Notify,
    resume: Notify,
    deleted: Notify,
    deletes: AtomicUsize,
    duplicate: bool,
}

impl Storage for PausedCommit {
    fn create(
        &self,
        meta: SessionMeta,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), storage::Error>> {
        Box::pin(async move {
            if let Some(gate) = &self.create_gate {
                if gate.duplicate {
                    self.inner.create(meta.clone(), resume.clone()).await?;
                }
                gate.entered.notify_one();
                gate.resume.notified().await;
            }
            self.inner.create(meta, resume).await
        })
    }
    fn commit<'a>(
        &'a self,
        meta: SessionMeta,
        events: &'a [Event],
        acceptance: Option<storage::PromptAcceptance>,
    ) -> BoxFuture<'a, Result<(), storage::Error>> {
        Box::pin(async move {
            if events.iter().any(self.pause_on) {
                self.entered.notify_one();
                self.resume.notified().await;
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
    fn page<'a>(
        &'a self,
        id: SessionId,
        after: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'a, Result<storage::Page, storage::Error>> {
        self.inner.page(id, after, limit)
    }
    fn tail(
        &self,
        id: SessionId,
        count: usize,
    ) -> BoxFuture<'_, Result<Vec<Event>, storage::Error>> {
        self.inner.tail(id, count)
    }
    fn delete(&self, id: SessionId) -> BoxFuture<'_, Result<bool, storage::Error>> {
        Box::pin(async move {
            let result = self.inner.delete(id).await;
            if let Some(gate) = &self.create_gate {
                gate.deletes.fetch_add(1, Ordering::SeqCst);
                gate.deleted.notify_one();
            }
            result
        })
    }
    fn is_persistent(&self) -> bool {
        false
    }
}

struct CountingAgent(Arc<AtomicUsize>);

#[tokio::test]
async fn terminal_interruption_and_status_commit_together_before_notification() {
    for failed in [false, true] {
        let storage = Arc::new(PausedCommit {
            inner: Memory::new(NonZeroUsize::new(100).expect("capacity")),
            entered: Notify::new(),
            resume: Notify::new(),
            pause_on: |event| matches!(event.kind, EventKind::TurnInterrupted { .. }),
            create_gate: None,
        });
        let meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
        let id = meta.id;
        storage.create(meta.clone(), None).await.expect("create");
        let (sender, mut completions) = mpsc::channel(2);
        let session = Session::with_storage(
            meta,
            Limits::default(),
            Some(sender),
            Some(storage.clone()),
            true,
        );
        let (updates, inbox) = mpsc::channel(16);
        session.listen(inbox);
        session
            .ready(Box::new(PendingAgent { _updates: updates }))
            .await
            .expect("ready");
        let turn = session.prompt(vec!["wait".into()]).await.expect("prompt");
        let cursor = session.meta().revision;
        let mut attachment = session
            .attach(AttachOptions {
                replay: 0,
                cursor: None,
            })
            .await
            .expect("attach");
        let worker = session.clone();
        let task = tokio::spawn(async move {
            if failed {
                worker.fail("offline".into()).await;
            } else {
                worker.shutdown().await;
            }
        });
        storage.entered.notified().await;
        assert_eq!(session.meta().status, SessionStatus::Working);
        assert!(completions.try_recv().is_err());
        assert!(
            storage
                .page(id, Some(cursor), 10)
                .await
                .expect("page")
                .events
                .is_empty()
        );
        storage.resume.notify_one();
        task.await.expect("terminal operation");
        let expected = if failed {
            SessionStatus::Failed
        } else {
            SessionStatus::Stopped
        };
        let completion = completions.recv().await.expect("completion");
        assert_eq!(completion.session.status, expected);
        assert!(matches!(
            completion.outcome,
            crate::session::CompletionOutcome::Interrupted(_)
        ));
        let page = storage.page(id, Some(cursor), 10).await.expect("history");
        assert_eq!(page.events.len(), 2);
        assert!(
            matches!(page.events[0].kind, EventKind::TurnInterrupted { turn: interrupted, .. } if interrupted == turn)
        );
        assert_eq!(page.events[0].cursor, cursor.next());
        assert_eq!(page.events[1].cursor, cursor.next().next());
        if failed {
            assert!(matches!(page.events[1].kind, EventKind::Failed { .. }));
        } else {
            assert!(matches!(
                page.events[1].kind,
                EventKind::StatusChanged {
                    status: SessionStatus::Stopped
                }
            ));
        }
        assert_eq!(
            storage
                .load(id)
                .await
                .expect("load")
                .expect("stored")
                .meta
                .status,
            expected
        );
        for event in page.events {
            assert_eq!(attachment.recv().await.expect("committed event"), event);
        }
        assert_eq!(
            attachment.recv().await,
            Err(crate::session::AttachError::Closed)
        );
    }
}

fn paused_create(duplicate: bool) -> Arc<PausedCommit> {
    Arc::new(PausedCommit {
        inner: Memory::new(NonZeroUsize::new(100).expect("history capacity")),
        entered: Notify::new(),
        resume: Notify::new(),
        pause_on: |_| false,
        create_gate: Some(CreateGate {
            duplicate,
            ..CreateGate::default()
        }),
    })
}

#[tokio::test]
async fn cancelled_or_timed_out_create_cleans_up_only_after_write_completes() {
    for cancellation in ["abort", "timeout", "shutdown"] {
        let storage = paused_create(false);
        let gate = storage.create_gate.as_ref().expect("create gate");
        let registry = Arc::new(Registry::with_storage(
            Arc::new(Catalog::with_defaults()),
            Arc::new(LocalHost::default()),
            Limits {
                startup_timeout: if cancellation == "timeout" {
                    std::time::Duration::from_millis(100)
                } else {
                    super::TIMEOUT
                },
                ..Limits::default()
            },
            storage.clone(),
        ));
        let creator = Arc::clone(&registry);
        let caller = tokio::spawn(async move { creator.create(request()).await });
        tokio::time::timeout(super::TIMEOUT, gate.entered.notified())
            .await
            .expect("create write entered");
        if cancellation == "timeout" {
            assert!(matches!(
                caller.await.expect("caller completed"),
                Err(CreateError::Timeout)
            ));
        } else if cancellation == "shutdown" {
            registry.shutdown().await;
            assert!(matches!(
                caller.await.expect("caller completed"),
                Err(CreateError::Stopping)
            ));
        } else {
            caller.abort();
            assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
        }
        assert_eq!(gate.deletes.load(Ordering::SeqCst), 0);
        assert!(storage.list().await.expect("no write yet").is_empty());
        gate.resume.notify_one();
        tokio::time::timeout(super::TIMEOUT, gate.deleted.notified())
            .await
            .expect("successful late write compensated");
        assert_eq!(gate.deletes.load(Ordering::SeqCst), 1);
        assert!(storage.list().await.expect("no orphan").is_empty());
        registry.shutdown().await;
    }
}

#[tokio::test]
async fn failed_create_does_not_delete_an_existing_record() {
    let storage = paused_create(true);
    let gate = storage.create_gate.as_ref().expect("create gate");
    let registry = Arc::new(Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    ));
    let creator = Arc::clone(&registry);
    let caller = tokio::spawn(async move { creator.create(request()).await });
    tokio::time::timeout(super::TIMEOUT, gate.entered.notified())
        .await
        .expect("create entered with conflicting record");
    let existing = storage.list().await.expect("existing record");
    gate.resume.notify_one();
    assert!(matches!(
        caller.await.expect("caller completed"),
        Err(CreateError::Storage(storage::Error::AlreadyExists(_)))
    ));
    assert_eq!(gate.deletes.load(Ordering::SeqCst), 0);
    assert_eq!(storage.list().await.expect("record preserved"), existing);
}

#[tokio::test]
async fn recovered_terminal_history_replays_in_order_then_closes() {
    for status in [
        SessionStatus::Stopped,
        SessionStatus::Failed,
        SessionStatus::Starting,
        SessionStatus::Idle,
        SessionStatus::Working,
    ] {
        let storage = Arc::new(LocalDb::open(":memory:").await.expect("database"));
        let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
        let id = meta.id;
        storage.create(meta.clone(), None).await.expect("create");
        for title in ["first", "second"] {
            let event = Event::new(
                id,
                meta.revision.next(),
                EventKind::TitleChanged {
                    title: title.into(),
                },
            );
            meta.apply_event(&event);
            storage
                .commit(meta.clone(), &[event], None)
                .await
                .expect("history");
        }
        meta.set_status(status);
        storage.commit(meta, &[], None).await.expect("status");
        let mut registry = Registry::with_storage(
            Arc::new(Catalog::with_defaults()),
            Arc::new(LocalHost::default()),
            Limits::default(),
            storage.clone(),
        );
        registry.recover().await.expect("recover");
        let session = registry.get(id).await.expect("session");
        let expected = storage
            .page(id, Some(Cursor::START), 100)
            .await
            .expect("history")
            .events;
        assert_eq!(
            expected.len(),
            if status == SessionStatus::Starting {
                3
            } else {
                2
            }
        );
        for options in [
            AttachOptions {
                replay: 100,
                cursor: None,
            },
            AttachOptions {
                replay: 0,
                cursor: Some(Cursor::START),
            },
        ] {
            let mut attachment = session.try_attach(options).await.expect("attach");
            for event in &expected {
                assert_eq!(attachment.recv().await.expect("replayed event"), *event);
            }
            if matches!(status, SessionStatus::Idle | SessionStatus::Working) {
                // A single poll must wait for live updates, not report a closed stream.
                let next = attachment.recv();
                futures::pin_mut!(next);
                assert!(futures::poll!(next).is_pending());
            } else {
                assert_eq!(
                    tokio::time::timeout(super::TIMEOUT, attachment.recv())
                        .await
                        .expect("terminal EOF"),
                    Err(super::AttachError::Closed)
                );
                use futures::StreamExt;
                let stream = attachment.into_stream();
                futures::pin_mut!(stream);
                assert!(stream.next().await.is_none());
            }
        }
    }
}

#[async_trait]
impl Agent for CountingAgent {
    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }
    async fn shutdown(&mut self) {}
}

fn mode_snapshot(value: &str) -> acp::NewSessionResponse {
    serde_json::from_value(serde_json::json!({
        "sessionId": "backend-id",
        "configOptions": [{
            "id": "mode", "name": "Mode", "category": "mode", "type": "select",
            "currentValue": value,
            "options": [
                {"value": "workspace-write", "name": "Workspace access"},
                {"value": "agent", "name": "Auto review"},
                {"value": "agent-full-access", "name": "Full access"}
            ]
        }],
        "modes": {
            "currentModeId": value,
            "availableModes": [
                {"id": "workspace-write", "name": "Workspace access"},
                {"id": "agent", "name": "Auto review"},
                {"id": "agent-full-access", "name": "Full access"}
            ]
        }
    }))
    .expect("ACP session snapshot")
}

fn initialization_raw(load_session: bool) -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": 1,
        "agentCapabilities": {"loadSession": load_session},
        "fixtureCapability": {"restored": load_session}
    })
}

struct RestoredSnapshot {
    _updates: mpsc::Sender<Update>,
    info: Option<acp::NewSessionResponse>,
}

#[async_trait]
impl Agent for RestoredSnapshot {
    fn initialization(&self) -> Option<acp::InitializeResponse> {
        Some(serde_json::from_value(initialization_raw(true)).expect("ACP initialization"))
    }
    fn initialization_raw(&self) -> Option<serde_json::Value> {
        Some(initialization_raw(true))
    }
    fn session_info(&self) -> Option<acp::NewSessionResponse> {
        self.info.clone()
    }
    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        Ok(())
    }
    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }
    async fn shutdown(&mut self) {}
}

struct RestoreSnapshotDriver(Arc<AtomicUsize>);

#[async_trait]
impl Driver for RestoreSnapshotDriver {
    fn kind(&self) -> Kind {
        Kind::Embedded
    }
    async fn launch(&self, _: LaunchRequest) -> Result<Box<dyn Agent>, agent::Error> {
        unreachable!("fixture only loads recovered sessions")
    }
    async fn restore(
        &self,
        request: LaunchRequest,
        backend_session_id: String,
    ) -> Result<Box<dyn Agent>, agent::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        assert!(backend_session_id.is_empty());
        request
            .updates
            .send(Update::Acp(acp::SessionUpdate::AgentMessageChunk(
                acp::ContentChunk::new("replayed history".into()),
            )))
            .await
            .expect("replay queue");
        request
            .updates
            .send(Update::Acp(acp::SessionUpdate::ConfigOptionUpdate(
                acp::ConfigOptionUpdate::new(
                    mode_snapshot("workspace-write")
                        .config_options
                        .expect("options"),
                ),
            )))
            .await
            .expect("replayed configuration");
        Ok(Box::new(RestoredSnapshot {
            _updates: request.updates,
            info: Some(mode_snapshot("agent")),
        }))
    }
}

#[tokio::test]
async fn restored_snapshot_replaces_saved_mode_and_publishes_committed_actual_state() {
    let storage = Arc::new(Memory::new(
        NonZeroUsize::new(100).expect("history capacity"),
    ));
    let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    let id = meta.id;
    let workspace = tempfile::tempdir().expect("workspace");
    meta.cwd = Some(workspace.path().to_owned());
    meta.set_status(SessionStatus::Idle);
    meta.initialization =
        Some(serde_json::from_value(initialization_raw(false)).expect("old initialization"));
    meta.initialization_raw = Some(initialization_raw(false));
    let mut saved_info = mode_snapshot("workspace-write");
    saved_info.session_id = id.to_string().into();
    meta.acp = Some(saved_info);
    storage.create(meta, None).await.expect("saved session");
    let mut catalog = Catalog::new();
    catalog
        .insert(
            AgentSpec::embedded(),
            Arc::new(RestoreSnapshotDriver(Arc::new(AtomicUsize::new(0)))),
        )
        .expect("fixture driver");
    let mut registry = Registry::with_storage(
        Arc::new(catalog),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    );
    registry.recover().await.expect("recover");
    let session = registry.get(id).await.expect("recovered session");
    let mut live = session
        .attach(AttachOptions {
            replay: 0,
            cursor: None,
        })
        .await
        .expect("attach before restore");
    registry.ensure_restored(&session).await.expect("restore");

    let meta = registry
        .list()
        .await
        .pop()
        .expect("GUI-facing session metadata");
    let info = serde_json::to_value(meta.acp.as_ref().expect("session snapshot")).expect("JSON");
    assert_eq!(info["sessionId"], id.to_string());
    assert_eq!(info["configOptions"][0]["currentValue"], "agent");
    assert_eq!(info["modes"]["currentModeId"], "agent");
    assert!(
        meta.initialization
            .as_ref()
            .expect("initialization")
            .agent_capabilities
            .load_session
    );
    assert_eq!(meta.initialization_raw, Some(initialization_raw(true)));
    let events = storage
        .page(id, None, 10)
        .await
        .expect("committed events")
        .events;
    assert_eq!(events.len(), 2, "load replay must not duplicate history");
    assert!(matches!(
        events[0].kind,
        EventKind::Acp(acp::SessionUpdate::ConfigOptionUpdate(_))
    ));
    assert!(matches!(
        events[1].kind,
        EventKind::Acp(acp::SessionUpdate::CurrentModeUpdate(_))
    ));
    assert_eq!(events[0].cursor, Cursor::START.next());
    assert_eq!(events[1].cursor, meta.revision);
    assert_eq!(
        storage.load(id).await.expect("load").expect("record").meta,
        meta
    );
    for event in &events {
        assert_eq!(
            tokio::time::timeout(super::TIMEOUT, live.recv())
                .await
                .expect("live update")
                .expect("event"),
            *event
        );
    }
    registry.shutdown().await;
}

#[tokio::test]
async fn restore_barrier_retains_updates_after_the_load_response() {
    let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    meta.set_status(SessionStatus::Idle);
    meta.acp = Some(mode_snapshot("workspace-write"));
    let storage = Arc::new(Memory::new(
        NonZeroUsize::new(100).expect("history capacity"),
    ));
    let id = meta.id;
    storage
        .create(meta.clone(), None)
        .await
        .expect("saved session");
    let session =
        Session::with_storage(meta, Limits::default(), None, Some(storage.clone()), false);
    let (updates, inbox) = mpsc::channel(10);
    session.listen(inbox);
    let restoring = Arc::clone(&session.restore_gate).lock_owned().await;
    let (ack, drained) = tokio::sync::oneshot::channel();
    updates
        .send(Update::Barrier(ack))
        .await
        .expect("restore barrier");
    drained.await.expect("replay drained");
    let (committed, completion) = tokio::sync::oneshot::channel();
    updates
        .send(Update::Commit {
            update: acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(
                mode_snapshot("agent-full-access")
                    .config_options
                    .expect("options"),
            )),
            committed,
        })
        .await
        .expect("post-load update");
    session
        .restore_agent(
            Box::new(RestoredSnapshot {
                _updates: updates,
                info: Some(mode_snapshot("agent")),
            }),
            restoring,
        )
        .await
        .expect("commit load response");
    tokio::time::timeout(super::TIMEOUT, completion)
        .await
        .expect("later update commits")
        .expect("ack");
    let current = session.meta();
    let info = serde_json::to_value(current.acp.expect("snapshot")).expect("JSON");
    assert_eq!(
        info["configOptions"][0]["currentValue"],
        "agent-full-access"
    );
    let events = storage.page(id, None, 10).await.expect("events").events;
    assert_eq!(events.len(), 3);
    assert_eq!(
        events.last().expect("later update").cursor,
        current.revision
    );
    session.shutdown().await;
}

#[tokio::test]
async fn restore_commits_initialization_changes_without_inventing_history_events() {
    let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    meta.set_status(SessionStatus::Idle);
    let storage = Arc::new(Memory::new(
        NonZeroUsize::new(100).expect("history capacity"),
    ));
    let id = meta.id;
    storage
        .create(meta.clone(), None)
        .await
        .expect("saved session");
    let session =
        Session::with_storage(meta, Limits::default(), None, Some(storage.clone()), false);
    let (updates, _) = mpsc::channel(1);
    session
        .restore_agent(
            Box::new(RestoredSnapshot {
                _updates: updates,
                info: None,
            }),
            Arc::clone(&session.restore_gate).lock_owned().await,
        )
        .await
        .expect("restore initialization");
    let current = session.meta();
    assert!(session.is_restored());
    assert_eq!(current.revision, Cursor::START);
    assert_eq!(current.initialization_raw, Some(initialization_raw(true)));
    assert_eq!(
        storage.load(id).await.expect("load").expect("record").meta,
        current
    );
    assert!(
        storage
            .page(id, None, 10)
            .await
            .expect("history")
            .events
            .is_empty()
    );
    session.shutdown().await;
}

#[tokio::test]
async fn cancelled_restore_commit_retains_gate_and_retry_uses_one_agent() {
    for timeout in [false, true] {
        let storage = Arc::new(PausedCommit {
            inner: Memory::new(NonZeroUsize::new(100).expect("history capacity")),
            entered: Notify::new(),
            resume: Notify::new(),
            create_gate: None,
            pause_on: |event| {
                matches!(
                    event.kind,
                    EventKind::Acp(acp::SessionUpdate::ConfigOptionUpdate(_))
                )
            },
        });
        let workspace = tempfile::tempdir().expect("workspace");
        let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
        let id = meta.id;
        meta.cwd = Some(workspace.path().to_owned());
        meta.set_status(SessionStatus::Idle);
        meta.acp = Some(mode_snapshot("workspace-write"));
        storage.create(meta, None).await.expect("saved session");
        let restores = Arc::new(AtomicUsize::new(0));
        let mut catalog = Catalog::new();
        catalog
            .insert(
                AgentSpec::embedded(),
                Arc::new(RestoreSnapshotDriver(Arc::clone(&restores))),
            )
            .expect("fixture driver");
        let mut registry = Registry::with_storage(
            Arc::new(catalog),
            Arc::new(LocalHost::default()),
            Limits {
                startup_timeout: if timeout {
                    std::time::Duration::from_millis(100)
                } else {
                    super::TIMEOUT
                },
                ..Limits::default()
            },
            storage.clone(),
        );
        registry.recover().await.expect("recover");
        let registry = Arc::new(registry);
        let session = registry.get(id).await.expect("recovered session");
        let caller = {
            let registry = Arc::clone(&registry);
            let session = Arc::clone(&session);
            tokio::spawn(async move { registry.ensure_restored(&session).await })
        };
        tokio::time::timeout(super::TIMEOUT, storage.entered.notified())
            .await
            .expect("restore commit entered");
        if timeout {
            assert!(matches!(
                caller.await.expect("restore caller"),
                Err(CreateError::Timeout)
            ));
        } else {
            caller.abort();
            assert!(caller.await.expect_err("aborted caller").is_cancelled());
        }
        assert!(!session.is_restored());
        assert!(
            session.restore_gate.try_lock().is_err(),
            "commit owns restore gate"
        );
        let mut retry = Box::pin(registry.ensure_restored(&session));
        assert!(futures::poll!(retry.as_mut()).is_pending());
        assert_eq!(
            restores.load(Ordering::SeqCst),
            1,
            "retry must wait for the original agent"
        );
        storage.resume.notify_one();
        tokio::time::timeout(super::TIMEOUT, retry)
            .await
            .expect("retry completes after commit")
            .expect("restore succeeds");
        assert!(session.is_restored());
        assert_eq!(restores.load(Ordering::SeqCst), 1);
        assert!(session.restore_gate.try_lock().is_ok());
        let current = session.meta();
        assert_eq!(
            storage.load(id).await.expect("load").expect("record").meta,
            current
        );
        let events = storage
            .page(id, None, 10)
            .await
            .expect("restore events")
            .events;
        assert_eq!(events.len(), 2, "one restore state publication");
        session
            .accept_prompt(basr::model::PromptSubmission::new(acp::PromptRequest::new(
                id.to_string(),
                vec!["hello".into()],
            )))
            .await
            .expect("the restored agent is installed and accepts prompts");
        registry.shutdown().await;
    }
}

#[tokio::test]
async fn aborted_prompt_during_commit_keeps_revision_contiguous_and_dispatches_once() {
    let storage = Arc::new(PausedCommit {
        inner: Memory::new(NonZeroUsize::new(100).expect("history capacity")),
        entered: Notify::new(),
        resume: Notify::new(),
        create_gate: None,
        pause_on: |event| matches!(event.kind, EventKind::TurnStarted { .. }),
    });
    let meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    storage
        .create(meta.clone(), None)
        .await
        .expect("create record");
    let id = meta.id;
    let session = Session::with_storage(meta, Limits::default(), None, Some(storage.clone()), true);
    let dispatched = Arc::new(AtomicUsize::new(0));
    session
        .ready(Box::new(CountingAgent(Arc::clone(&dispatched))))
        .await
        .expect("ready");
    let revision = session.meta().revision;
    let caller = {
        let session = Arc::clone(&session);
        tokio::spawn(async move {
            session
                .accept_prompt(basr::model::PromptSubmission::new(acp::PromptRequest::new(
                    id.to_string(),
                    vec!["hello".into()],
                )))
                .await
        })
    };
    tokio::time::timeout(super::TIMEOUT, storage.entered.notified())
        .await
        .expect("turn commit entered");
    caller.abort();
    let _ = caller.await;
    storage.resume.notify_one();
    tokio::time::timeout(super::TIMEOUT, async {
        while dispatched.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("turn dispatched after commit");
    let current = session.meta();
    let stored = storage.load(id).await.expect("load").expect("record");
    assert_eq!(current.revision, stored.meta.revision);
    let events = storage
        .page(id, Some(revision), 10)
        .await
        .expect("events")
        .events;
    assert!(!events.is_empty());
    assert!(
        events
            .iter()
            .enumerate()
            .all(|(index, event)| event.cursor.raw() == revision.raw() + index as u64 + 1)
    );
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    session.shutdown().await;
}

#[tokio::test]
async fn durable_sessions_recover_metadata_history_and_lazy_embedded_backend() {
    let dir = tempfile::tempdir().expect("storage directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private storage directory");
    }
    let path = dir.path().join("sessions.sqlite");
    let open = || async {
        Registry::with_storage(
            Arc::new(Catalog::with_defaults()),
            Arc::new(LocalHost::default()),
            Limits::default(),
            Arc::new(LocalDb::open(&path).await.expect("open storage")),
        )
    };
    let registry = open().await;
    let session = registry.create(request()).await.expect("create");
    let id = session.id();
    session
        .prompt(vec![ContentBlock::from("first")])
        .await
        .expect("prompt");
    let first_cursor = session.meta().revision;
    let mut events = session
        .try_attach(super::AttachOptions {
            replay: 0,
            cursor: Some(first_cursor),
        })
        .await
        .expect("attach after first prompt");
    tokio::time::timeout(super::TIMEOUT, async {
        loop {
            if matches!(
                events.recv().await.expect("event").kind,
                basr::model::EventKind::TurnFinished { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("turn finished");
    let before = session.meta().revision;
    registry.shutdown().await;
    drop(events);
    drop(session);
    drop(registry);

    let mut registry = open().await;
    registry.recover().await.expect("rebuild records");
    let session = registry.get(id).await.expect("recovered session");
    assert_eq!(session.meta().status, SessionStatus::Idle);
    assert_eq!(session.meta().revision, before);
    let page = session
        .try_attach(super::AttachOptions {
            replay: 0,
            cursor: Some(Cursor::START),
        })
        .await
        .expect("full persisted history");
    drop(page);
    assert!(!session.is_restored());
    registry
        .ensure_restored(&session)
        .await
        .expect("restore embedded backend");
    assert!(session.is_restored());
    session
        .prompt(vec![ContentBlock::from("second")])
        .await
        .expect("second turn");
    assert!(session.meta().revision > before);
    registry.remove(id).await.expect("delete persisted session");
    let db = LocalDb::open(&path).await.expect("reopen db");
    assert!(db.load(id).await.expect("load deleted session").is_none());
}

#[tokio::test]
async fn memory_attach_without_cursor_replays_the_retained_tail_after_eviction() {
    let capacity = NonZeroUsize::new(2).expect("nonzero history");
    let registry = Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits {
            history_capacity: capacity,
            ..Limits::default()
        },
        Arc::new(Memory::new(capacity)),
    );
    let session = registry.create(request()).await.expect("create");
    session
        .prompt(vec![ContentBlock::from("text")])
        .await
        .expect("prompt");
    let mut live = session
        .try_attach(super::AttachOptions {
            replay: 0,
            cursor: Some(session.meta().revision),
        })
        .await
        .expect("listen");
    tokio::time::timeout(super::TIMEOUT, async {
        loop {
            if matches!(
                live.recv().await.expect("event").kind,
                EventKind::TurnFinished { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("turn finished");
    let mut replay = session
        .try_attach(super::AttachOptions {
            replay: 100,
            cursor: None,
        })
        .await
        .expect("retained tail must not expire");
    let first = replay.recv().await.expect("first retained event");
    let second = replay.recv().await.expect("second retained event");
    assert!(first.cursor < second.cursor);
    assert_eq!(second.cursor, session.meta().revision);
    registry.shutdown().await;
}

#[tokio::test]
async fn durable_attach_backfills_a_lagged_live_receiver_from_sqlite() {
    let dir = tempfile::tempdir().expect("storage directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private storage directory");
    }
    let registry = Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits {
            live_capacity: NonZeroUsize::new(2).expect("nonzero live buffer"),
            ..Limits::default()
        },
        Arc::new(
            LocalDb::open(dir.path().join("sessions.sqlite"))
                .await
                .expect("database"),
        ),
    );
    let session = registry.create(request()).await.expect("create");
    let previous = session.meta().revision;
    let mut attachment = session
        .try_attach(super::AttachOptions {
            replay: 0,
            cursor: Some(previous),
        })
        .await
        .expect("attach");
    for archived in [true, false].into_iter().cycle().take(20) {
        session
            .try_set_archived(archived)
            .await
            .expect("save archive event");
    }
    for offset in 1..=20 {
        let event = tokio::time::timeout(super::TIMEOUT, attachment.recv())
            .await
            .expect("not stuck")
            .expect("history backfill");
        assert_eq!(event.cursor.raw(), previous.raw() + offset);
    }
    registry.shutdown().await;
}

#[tokio::test]
async fn recovery_records_an_interrupted_turn_instead_of_silently_clearing_it() {
    let storage: Arc<dyn Storage> = Arc::new(LocalDb::open(":memory:").await.expect("database"));
    let limits = Limits::default();
    let meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    let id = meta.id;
    storage
        .create(meta.clone(), None)
        .await
        .expect("save session");
    let session = Session::with_storage(meta, limits, None, Some(Arc::clone(&storage)), true);
    let (updates, inbox) = mpsc::channel(limits.live_capacity.get());
    session.listen(inbox);
    session
        .ready(Box::new(PendingAgent { _updates: updates }))
        .await
        .expect("session ready");
    let turn = session
        .prompt(vec![ContentBlock::from("interrupted")])
        .await
        .expect("prompt accepted");
    let started = session.meta().revision;
    assert_eq!(session.meta().status, SessionStatus::Working);
    session.suspend().await;
    drop(session);

    let mut registry = Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        limits,
        Arc::clone(&storage),
    );
    registry.recover().await.expect("recover interrupted turn");
    let recovered = registry.get(id).await.expect("same session");
    assert_eq!(recovered.meta().status, SessionStatus::Idle);
    assert_eq!(recovered.meta().active_turn, None);
    assert_eq!(recovered.meta().revision, started.next());
    let event = storage
        .page(id, Some(started), 1)
        .await
        .expect("history")
        .events
        .into_iter()
        .next()
        .expect("interruption event");
    assert!(matches!(
        event.kind,
        EventKind::TurnInterrupted {
            turn: finished,
            reason: basr::model::InterruptionReason::DaemonRestarted
        } if finished == turn
    ));
}
