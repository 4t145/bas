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

use super::{Catalog, Limits, LocalHost, PendingAgent, Registry, Session, request};
use crate::{
    agent::{self, Agent},
    storage::{self, LocalDb, Memory, ResumeData, Storage, StoredSession},
};

/// Holds only the turn commit, so initialization can complete before the test starts.
struct PausedCommit {
    inner: Memory,
    entered: Notify,
    resume: Notify,
}

impl Storage for PausedCommit {
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
    ) -> BoxFuture<'a, Result<(), storage::Error>> {
        Box::pin(async move {
            if events
                .iter()
                .any(|event| matches!(event.kind, EventKind::TurnStarted { .. }))
            {
                self.entered.notify_one();
                self.resume.notified().await;
            }
            self.inner.commit(meta, events).await
        })
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
        self.inner.delete(id)
    }
    fn is_persistent(&self) -> bool {
        false
    }
}

struct CountingAgent(Arc<AtomicUsize>);

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

#[tokio::test]
async fn aborted_prompt_during_commit_keeps_revision_contiguous_and_dispatches_once() {
    let storage = Arc::new(PausedCommit {
        inner: Memory::new(NonZeroUsize::new(100).expect("history capacity")),
        entered: Notify::new(),
        resume: Notify::new(),
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
                .accept_prompt(acp::PromptRequest::new(
                    id.to_string(),
                    vec!["hello".into()],
                ))
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
        EventKind::TurnFinished {
            turn: finished,
            result: Err(_)
        } if finished == turn
    ));
}
