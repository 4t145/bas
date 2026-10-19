use bas::acp;
mod archive;
mod notices;
mod persistence;
mod recovery;

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use bas::model::{
    ContentBlock, Cursor, Event, EventKind, Owner, PromptRejection, PromptRejectionKind, SessionId,
    SessionMeta, SessionStatus, StopReason, TurnId,
};
use tokio::sync::{Notify, mpsc};

use super::{
    AttachError, AttachOptions, CreateError, CreateRequest, HistoryError, Limits, Registry,
    Session, SessionError, SubmissionError, history::Ring,
};
use crate::agent::{
    self, Agent, AgentSpec, Catalog, Driver, Kind, LaunchRequest, LocalHost, TurnUpdate, Update,
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn request() -> CreateRequest {
    CreateRequest {
        agent: "embedded".into(),
        owner: Owner::current(),
        cwd: None,
        title: None,
    }
}

fn registry() -> Registry {
    Registry::new(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
    )
}

struct PendingAgent {
    _updates: mpsc::Sender<Update>,
}

#[async_trait]
impl Agent for PendingAgent {
    fn prompt(&mut self, _turn: TurnId, _request: acp::PromptRequest) -> Result<(), agent::Error> {
        Ok(())
    }
    fn cancel(&mut self, _request: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }
    async fn shutdown(&mut self) {}
}

async fn pending_session(limits: Limits) -> Arc<Session> {
    let session = Session::new(
        SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current()),
        limits,
        None,
    );
    let (updates, inbox) = mpsc::channel(limits.live_capacity.get());
    session.listen(inbox);
    session
        .ready(Box::new(PendingAgent { _updates: updates }))
        .await
        .expect("session starts");
    session
}

#[tokio::test]
async fn startup_overlay_survives_content_eviction_before_ready() {
    struct InitialSnapshot;
    #[async_trait]
    impl Agent for InitialSnapshot {
        fn session_info(&self) -> Option<acp::NewSessionResponse> {
            Some(serde_json::from_value(serde_json::json!({
                "sessionId": "backend",
                "configOptions": [{
                    "id": "enabled", "name": "Enabled", "type": "boolean", "currentValue": false
                }],
                "modes": {
                    "currentModeId": "ask",
                    "availableModes": [{"id": "ask", "name": "Ask"}, {"id": "code", "name": "Code"}]
                }
            })).expect("initial snapshot"))
        }
        fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
            Ok(())
        }
        fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
            Ok(())
        }
        async fn shutdown(&mut self) {}
    }
    let session = Session::new(
        SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current()),
        Limits {
            history_capacity: NonZeroUsize::new(1).expect("one event"),
            ..Limits::default()
        },
        None,
    );
    session
        .apply(Update::Acp(
            serde_json::from_value(serde_json::json!({
                "sessionUpdate": "config_option_update",
                "configOptions": [{
                    "id": "enabled", "name": "Enabled", "type": "boolean", "currentValue": true
                }]
            }))
            .expect("startup config"),
        ))
        .await;
    session
        .apply(Update::Acp(acp::SessionUpdate::CurrentModeUpdate(
            acp::CurrentModeUpdate::new("code"),
        )))
        .await;
    session
        .apply(Update::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("startup content evicts configuration".into()),
        )))
        .await;
    assert_eq!(session.page(None, 10).expect("history").events.len(), 1);
    session
        .ready(Box::new(InitialSnapshot))
        .await
        .expect("ready");
    let meta = session.meta();
    let snapshot = serde_json::to_value(meta.acp.expect("ACP state")).expect("JSON");
    assert_eq!(snapshot["configOptions"][0]["currentValue"], true);
    assert_eq!(snapshot["modes"]["currentModeId"], "code");
    assert_eq!(
        meta.revision,
        session.page(None, 1).expect("history").events[0].cursor
    );
    assert_eq!(
        session
            .visit_history(Some(meta.revision.next()), 1, |_, _| panic!(
                "future cursor"
            ))
            .await,
        Err(HistoryError::Future {
            requested: meta.revision.next(),
            latest: meta.revision
        })
    );
    session.shutdown().await;
}

fn options(replay: usize) -> AttachOptions {
    AttachOptions {
        replay,
        cursor: None,
    }
}
fn message(cursor: u64) -> Event {
    Event::new(
        SessionId::new(),
        Cursor::from_raw(cursor),
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("message".into()),
        )),
    )
}

async fn finish(attachment: &mut super::Attachment) -> Result<StopReason, acp::Error> {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let EventKind::TurnFinished { result, .. } = attachment
                .recv()
                .await
                .expect("turn stream remains open")
                .kind
            {
                return result.map(|response| response.stop_reason);
            }
        }
    })
    .await
    .expect("turn finishes")
}

#[test]
fn ring_pages_and_reports_eviction_with_sparse_cursors() {
    let mut ring = Ring::new(NonZeroUsize::new(2).expect("nonzero"));
    ring.push(message(1));
    ring.push(message(10));
    ring.push(message(20));
    assert!(matches!(
        ring.page(Some(Cursor::START), 2),
        Err(HistoryError::Expired { .. })
    ));
    let page = ring
        .page(Some(Cursor::from_raw(1)), 1)
        .expect("only the consumed event was evicted");
    assert_eq!(page.events[0].cursor.raw(), 10);
    assert_eq!(page.next, Some(Cursor::from_raw(10)));
    assert_eq!(ring.page(page.next, 10).expect("last page").next, None);
    assert_eq!(ring.page(None, 10).expect("current window").events.len(), 2);
    assert_eq!(ring.page(None, 0), Err(HistoryError::EmptyPage));
}

#[tokio::test]
async fn expired_and_future_cursors_are_explicit_errors() {
    let session = pending_session(Limits {
        history_capacity: NonZeroUsize::new(2).expect("nonzero"),
        ..Limits::default()
    })
    .await;
    for i in 0..5 {
        session
            .set_title(&format!("title {i}"))
            .await
            .expect("rename");
    }
    assert!(matches!(
        session
            .attach(AttachOptions {
                cursor: Some(Cursor::START),
                replay: 0
            })
            .await,
        Err(HistoryError::Expired { .. })
    ));
    assert!(matches!(
        session.page(Some(Cursor::from_raw(100)), 1),
        Err(HistoryError::Future { .. })
    ));
    assert_eq!(
        session.page(None, 10).expect("current window").events.len(),
        2
    );
    let mut live = session.attach(options(0)).await.expect("live subscription");
    session.set_title("next").await.expect("rename");
    assert_eq!(live.recv().await.expect("live event").cursor.raw(), 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_publication_matches_history_and_metadata_order() {
    let session = pending_session(Limits::default()).await;
    let mut live = session.attach(options(0)).await.expect("attach");
    let mut workers = Vec::new();
    for worker in 0..4 {
        let session = Arc::clone(&session);
        workers.push(tokio::spawn(async move {
            for i in 0..100 {
                session
                    .set_title(&format!("{worker}/{i}"))
                    .await
                    .expect("rename");
            }
        }));
    }
    for worker in workers {
        worker.await.expect("writer completed");
    }
    let history = session
        .page(Some(Cursor::from_raw(1)), 500)
        .expect("history")
        .events;
    for expected in &history {
        assert_eq!(&live.recv().await.expect("live event"), expected);
    }
    assert_eq!(history.len(), 400);
    assert!(
        history
            .windows(2)
            .all(|pair| pair[0].cursor < pair[1].cursor)
    );
    let EventKind::TitleChanged { title } = &history.last().expect("events").kind else {
        panic!("expected title");
    };
    assert_eq!(session.meta().title.as_ref(), Some(title));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_and_live_handoff_neither_duplicate_nor_skip_events() {
    let session = pending_session(Limits::default()).await;
    let writer = Arc::clone(&session);
    let task = tokio::spawn(async move {
        for i in 0..300 {
            writer.set_title(&format!("{i}")).await.expect("rename");
            tokio::task::yield_now().await;
        }
    });
    let mut attachment = session
        .attach(AttachOptions {
            cursor: Some(Cursor::START),
            replay: 1,
        })
        .await
        .expect("attach");
    for expected in 1..=301 {
        let event = tokio::time::timeout(TIMEOUT, attachment.recv())
            .await
            .expect("no gap")
            .expect("event");
        assert_eq!(event.cursor.raw(), expected);
    }
    task.await.expect("writer completed");
}

#[tokio::test]
async fn overlapping_turns_do_not_commit_rejected_prompts() {
    let session = pending_session(Limits::default()).await;
    assert!(matches!(
        session.prompt(Vec::new()).await,
        Err(SubmissionError::Rejected(PromptRejection {
            kind: PromptRejectionKind::InvalidRequest,
            ..
        }))
    ));
    let first = session
        .prompt(vec![ContentBlock::from("first")])
        .await
        .expect("first accepted");
    assert_eq!(session.meta().status, SessionStatus::Working);
    assert!(matches!(
        session.prompt(vec![ContentBlock::from("rejected")]).await,
        Err(SubmissionError::Rejected(PromptRejection {
            kind: PromptRejectionKind::Busy,
            ..
        }))
    ));
    session
        .apply(Update::Turn {
            turn: first,
            update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::EndTurn))),
        })
        .await;
    let second = session
        .prompt(vec![ContentBlock::from("second")])
        .await
        .expect("second accepted");
    session
        .apply(Update::Turn {
            turn: first,
            update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::Cancelled))),
        })
        .await;
    assert_eq!(session.meta().status, SessionStatus::Working);
    session
        .apply(Update::Turn {
            turn: second,
            update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::EndTurn))),
        })
        .await;
    let texts: Vec<_> = session
        .page(None, 100)
        .expect("history")
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Acp(acp::SessionUpdate::UserMessageChunk(chunk)) => match &chunk.content {
                ContentBlock::Text(text) => Some(text.text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["first", "second"]);
}

#[tokio::test]
async fn undispatched_cancellation_is_an_interruption_and_session_can_continue() {
    let session = pending_session(Limits::default()).await;
    let turn = session
        .prompt(vec!["cancel before dispatch".into()])
        .await
        .expect("prompt");
    session
        .apply(Update::Turn {
            turn,
            update: TurnUpdate::CancelledBeforeDispatch,
        })
        .await;
    assert_eq!(session.meta().status, SessionStatus::Idle);
    assert_eq!(session.meta().active_turn, None);
    let history = session.page(None, 100).expect("history");
    assert!(matches!(
        history.events.last().expect("interruption").kind,
        EventKind::TurnInterrupted {
            reason: bas::model::InterruptionReason::CancelledBeforeDispatch,
            ..
        }
    ));
    assert!(
        !history
            .events
            .iter()
            .any(|event| matches!(event.kind, EventKind::TurnFinished { .. }))
    );
    session
        .prompt(vec!["continue".into()])
        .await
        .expect("next prompt");
    session.shutdown().await;
}

#[tokio::test]
async fn shutdown_is_terminal_and_closes_existing_and_new_attachments() {
    let session = pending_session(Limits::default()).await;
    let turn = session
        .prompt(vec![ContentBlock::from("first")])
        .await
        .expect("prompt");
    let mut attachment = session.attach(options(0)).await.expect("attach");
    session.shutdown().await;
    assert!(matches!(
        attachment.recv().await.expect("interruption").kind,
        EventKind::TurnInterrupted {
            reason: bas::model::InterruptionReason::SessionStopped,
            ..
        }
    ));
    assert!(matches!(
        attachment.recv().await.expect("stop event").kind,
        EventKind::StatusChanged {
            status: SessionStatus::Stopped
        }
    ));
    assert_eq!(attachment.recv().await, Err(AttachError::Closed));
    assert_eq!(
        session
            .attach(options(0))
            .await
            .expect("terminal attachment")
            .recv()
            .await,
        Err(AttachError::Closed)
    );
    session
        .apply(Update::Turn {
            turn,
            update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::EndTurn))),
        })
        .await;
    assert!(matches!(
        session.prompt(vec![ContentBlock::from("late")]).await,
        Err(SubmissionError::Rejected(PromptRejection {
            kind: PromptRejectionKind::Unavailable,
            ..
        }))
    ));
    assert_eq!(session.meta().status, SessionStatus::Stopped);
    session.shutdown().await;
}

#[tokio::test]
async fn slow_live_subscribers_receive_lag_errors() {
    let session = pending_session(Limits {
        live_capacity: NonZeroUsize::new(2).expect("nonzero"),
        ..Limits::default()
    })
    .await;
    let mut attachment = session.attach(options(0)).await.expect("attach");
    for i in 0..10 {
        session.set_title(&format!("{i}")).await.expect("rename");
    }
    assert!(matches!(
        attachment.recv().await,
        Err(AttachError::Lagged(_))
    ));
}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
struct HangingDriver {
    entered: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}
#[async_trait]
impl Driver for HangingDriver {
    fn kind(&self) -> Kind {
        Kind::Embedded
    }
    async fn launch(&self, _request: LaunchRequest) -> Result<Box<dyn Agent>, agent::Error> {
        let _guard = Dropped(Arc::clone(&self.dropped));
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn pending_startup_is_cancelled_by_timeout_and_shutdown() {
    for timeout in [true, false] {
        let entered = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let mut catalog = Catalog::new();
        catalog
            .insert(
                AgentSpec::embedded(),
                Arc::new(HangingDriver {
                    entered: Arc::clone(&entered),
                    dropped: Arc::clone(&dropped),
                }),
            )
            .expect("unique backend");
        let registry = Arc::new(Registry::new(
            Arc::new(catalog),
            Arc::new(LocalHost::default()),
            Limits {
                startup_timeout: if timeout {
                    Duration::from_millis(20)
                } else {
                    TIMEOUT
                },
                ..Limits::default()
            },
        ));
        let creator = Arc::clone(&registry);
        let task = tokio::spawn(async move { creator.create(request()).await });
        tokio::time::timeout(TIMEOUT, entered.notified())
            .await
            .expect("launch started");
        if !timeout {
            registry.shutdown().await;
        }
        let result = task.await.expect("create task completed");
        assert!(matches!(
            result,
            Err(CreateError::Timeout | CreateError::Stopping)
        ));
        assert!(dropped.load(Ordering::SeqCst));
        assert!(registry.list().await.is_empty());
    }
}
