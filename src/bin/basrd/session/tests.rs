use basr::acp;
mod archive;

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use basr::model::{
    ContentBlock, Cursor, Event, EventKind, Owner, SessionId, SessionMeta, SessionStatus,
    StopReason, TurnId,
};
use tokio::sync::{Notify, mpsc};

use super::{
    AttachError, AttachOptions, CreateError, CreateRequest, HistoryError, Limits, Registry,
    Session, SessionError, history::Ring,
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

#[cfg(unix)]
#[tokio::test]
async fn registry_rejects_non_utf8_canonical_workspaces_including_symlinks() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, fs::symlink},
    };
    let directory = tempfile::tempdir().expect("directory");
    let invalid = directory
        .path()
        .join(OsString::from_vec(b"workspace-\xff".to_vec()));
    std::fs::create_dir(&invalid).expect("non-UTF8 directory");
    let link = directory.path().join("valid-link");
    symlink(&invalid, &link).expect("symlink");
    let registry = registry();
    for cwd in [invalid, link] {
        let mut request = request();
        request.cwd = Some(cwd);
        assert!(
            matches!(registry.create(request).await, Err(CreateError::Cwd(error))
            if error.kind() == std::io::ErrorKind::InvalidInput)
        );
        assert!(registry.list().await.is_empty());
    }
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

fn pending_session(limits: Limits) -> Arc<Session> {
    let session = Session::new(
        SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current()),
        limits,
        None,
    );
    let (updates, inbox) = mpsc::channel(limits.live_capacity.get());
    session.listen(inbox);
    session
        .ready(Box::new(PendingAgent { _updates: updates }))
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
    session.apply(Update::Acp(
        serde_json::from_value(serde_json::json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [{
                "id": "enabled", "name": "Enabled", "type": "boolean", "currentValue": true
            }]
        }))
        .expect("startup config"),
    ));
    session.apply(Update::Acp(acp::SessionUpdate::CurrentModeUpdate(
        acp::CurrentModeUpdate::new("code"),
    )));
    session.apply(Update::Acp(acp::SessionUpdate::AgentMessageChunk(
        acp::ContentChunk::new("startup content evicts configuration".into()),
    )));
    assert_eq!(session.page(None, 10).expect("history").events.len(), 1);
    session.ready(Box::new(InitialSnapshot)).expect("ready");
    let meta = session.meta();
    let snapshot = serde_json::to_value(meta.acp.expect("ACP state")).expect("JSON");
    assert_eq!(snapshot["configOptions"][0]["currentValue"], true);
    assert_eq!(snapshot["modes"]["currentModeId"], "code");
    assert_eq!(
        meta.revision,
        session.page(None, 1).expect("history").events[0].cursor
    );
    assert_eq!(
        session.visit_history(Some(meta.revision.next()), 1, |_, _| panic!(
            "future cursor"
        )),
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

async fn finish(attachment: &mut super::Attachment) -> Result<StopReason, String> {
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
async fn resume_replays_all_retained_events_then_follows_live() {
    let session = pending_session(Limits::default());
    for i in 0..5 {
        session
            .set_title(&format!("title {i}"))
            .expect("rename succeeds");
    }
    let mut attachment = session
        .attach(AttachOptions {
            cursor: Some(Cursor::START),
            replay: 2,
        })
        .expect("cursor retained");
    assert_eq!(attachment.cursor(), Cursor::START);
    for expected in 1..=6 {
        let event = attachment.recv().await.expect("all history is available");
        assert_eq!(event.cursor.raw(), expected);
        assert_eq!(attachment.cursor(), event.cursor);
    }
    session.set_title("live").expect("rename succeeds");
    assert_eq!(attachment.recv().await.expect("live event").cursor.raw(), 7);
}

#[tokio::test]
async fn expired_and_future_cursors_are_explicit_errors() {
    let session = pending_session(Limits {
        history_capacity: NonZeroUsize::new(2).expect("nonzero"),
        ..Limits::default()
    });
    for i in 0..5 {
        session.set_title(&format!("title {i}")).expect("rename");
    }
    assert!(matches!(
        session.attach(AttachOptions {
            cursor: Some(Cursor::START),
            replay: 0
        }),
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
    let mut live = session.attach(options(0)).expect("live subscription");
    session.set_title("next").expect("rename");
    assert_eq!(live.recv().await.expect("live event").cursor.raw(), 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_publication_matches_history_and_metadata_order() {
    let session = pending_session(Limits::default());
    let mut live = session.attach(options(0)).expect("attach");
    std::thread::scope(|scope| {
        for worker in 0..4 {
            let session = &session;
            scope.spawn(move || {
                for i in 0..100 {
                    session.set_title(&format!("{worker}/{i}")).expect("rename");
                }
            });
        }
    });
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
    let session = pending_session(Limits::default());
    let writer = Arc::clone(&session);
    let task = tokio::spawn(async move {
        for i in 0..300 {
            writer.set_title(&format!("{i}")).expect("rename");
            tokio::task::yield_now().await;
        }
    });
    let mut attachment = session
        .attach(AttachOptions {
            cursor: Some(Cursor::START),
            replay: 1,
        })
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
    let session = pending_session(Limits::default());
    assert!(matches!(
        session.prompt(Vec::new()),
        Err(SessionError::EmptyPrompt)
    ));
    let first = session
        .prompt(vec![ContentBlock::from("first")])
        .expect("first accepted");
    assert_eq!(session.meta().status, SessionStatus::Working);
    assert!(matches!(
        session.prompt(vec![ContentBlock::from("rejected")]),
        Err(SessionError::Busy)
    ));
    session.apply(Update::Turn {
        turn: first,
        update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::EndTurn))),
    });
    let second = session
        .prompt(vec![ContentBlock::from("second")])
        .expect("second accepted");
    session.apply(Update::Turn {
        turn: first,
        update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::Cancelled))),
    });
    assert_eq!(session.meta().status, SessionStatus::Working);
    session.apply(Update::Turn {
        turn: second,
        update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::EndTurn))),
    });
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
async fn shutdown_is_terminal_and_closes_existing_and_new_attachments() {
    let session = pending_session(Limits::default());
    let turn = session
        .prompt(vec![ContentBlock::from("first")])
        .expect("prompt");
    let mut attachment = session.attach(options(0)).expect("attach");
    session.shutdown().await;
    assert_eq!(finish(&mut attachment).await, Ok(StopReason::Cancelled));
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
            .expect("terminal attachment")
            .recv()
            .await,
        Err(AttachError::Closed)
    );
    session.apply(Update::Turn {
        turn,
        update: TurnUpdate::Finished(Ok(acp::PromptResponse::new(StopReason::EndTurn))),
    });
    assert!(matches!(
        session.prompt(vec![ContentBlock::from("late")]),
        Err(SessionError::Inactive(SessionStatus::Stopped))
    ));
    assert_eq!(session.meta().status, SessionStatus::Stopped);
    session.shutdown().await;
}

#[tokio::test]
async fn backend_failure_is_replayable_and_terminal() {
    let session = pending_session(Limits::default());
    session
        .prompt(vec![ContentBlock::from("first")])
        .expect("prompt");
    let mut attachment = session.attach(options(0)).expect("attach");
    session.apply(Update::Failed("connection lost".into()));
    assert_eq!(finish(&mut attachment).await, Err("connection lost".into()));
    assert_eq!(
        attachment.recv().await.expect("failure event").kind,
        EventKind::Failed {
            message: "connection lost".into()
        }
    );
    assert_eq!(attachment.recv().await, Err(AttachError::Closed));
    assert_eq!(session.meta().status, SessionStatus::Failed);
    assert!(matches!(
        session
            .page(None, 100)
            .expect("history")
            .events
            .last()
            .expect("failure")
            .kind,
        EventKind::Failed { .. }
    ));
}

#[tokio::test]
async fn slow_live_subscribers_receive_lag_errors() {
    let session = pending_session(Limits {
        live_capacity: NonZeroUsize::new(2).expect("nonzero"),
        ..Limits::default()
    });
    let mut attachment = session.attach(options(0)).expect("attach");
    for i in 0..10 {
        session.set_title(&format!("{i}")).expect("rename");
    }
    assert!(matches!(
        attachment.recv().await,
        Err(AttachError::Lagged(_))
    ));
}

#[tokio::test]
async fn detaching_preserves_the_turn_and_removing_releases_the_session() {
    let (completions, mut completed) = mpsc::channel(1);
    let registry = registry().with_completions(completions);
    let session = registry.create(request()).await.expect("create");
    let attachment = session.attach(options(0)).expect("attach");
    session
        .prompt(vec![ContentBlock::from("hello")])
        .expect("prompt");
    drop(attachment);
    assert_eq!(session.subscriber_count(), 0);
    let completion = tokio::time::timeout(TIMEOUT, completed.recv())
        .await
        .expect("turn finishes without an attachment")
        .expect("completion notification");
    assert_eq!(
        completion.result.expect("successful turn").stop_reason,
        StopReason::EndTurn
    );
    let page = session.page(None, 100).expect("history");
    assert!(
        page.events
            .iter()
            .any(|event| matches!(&event.kind, EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(chunk)) if matches!(&chunk.content, ContentBlock::Text(text) if text.text.contains("echo agent"))))
    );
    let weak = Arc::downgrade(&session);
    registry.remove(session.id()).await.expect("remove");
    drop(session);
    assert!(weak.upgrade().is_none());
    assert!(registry.list().await.is_empty());
}

#[tokio::test]
async fn completion_snapshots_do_not_retain_sessions() {
    let (completions, mut receiver) = mpsc::channel(1);
    let registry = registry().with_completions(completions);
    let session = registry.create(request()).await.expect("create");
    let mut attachment = session.attach(options(0)).expect("attach");
    session
        .prompt(vec![ContentBlock::from("hello")])
        .expect("prompt");
    finish(&mut attachment).await.expect("turn succeeds");
    let weak = Arc::downgrade(&session);
    registry.remove(session.id()).await.expect("remove");
    drop(session);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        receiver
            .recv()
            .await
            .expect("completion snapshot")
            .result
            .expect("success")
            .stop_reason,
        StopReason::EndTurn
    );
}

#[tokio::test]
async fn foreground_viewers_suppress_only_their_session_and_release_without_retaining_it() {
    let (completions, mut receiver) = mpsc::channel(8);
    let registry = registry().with_completions(completions);
    let active = registry.create(request()).await.expect("active session");
    let other = registry.create(request()).await.expect("other session");
    let first_window = active.foreground();
    let second_window = active.foreground();
    drop(first_window);
    let mut history = active.attach(options(0)).expect("history");
    active
        .prompt(vec!["hello".into()])
        .expect("foreground turn");
    finish(&mut history).await.expect("turn finishes normally");
    assert!(receiver.try_recv().is_err());
    other.prompt(vec!["hello".into()]).expect("background turn");
    assert_eq!(
        tokio::time::timeout(TIMEOUT, receiver.recv())
            .await
            .expect("background notification")
            .expect("completion")
            .session
            .id,
        other.id()
    );
    drop(second_window);
    active.prompt(vec!["hello".into()]).expect("unfocused turn");
    assert_eq!(
        tokio::time::timeout(TIMEOUT, receiver.recv())
            .await
            .expect("notification restored")
            .expect("completion")
            .session
            .id,
        active.id()
    );
    let foreground = active.foreground();
    let weak = Arc::downgrade(&active);
    registry.remove(active.id()).await.expect("remove");
    drop(active);
    assert!(weak.upgrade().is_none());
    drop(foreground);
    registry.shutdown().await;
}

#[tokio::test]
async fn registry_shutdown_closes_streams_and_rejects_new_sessions() {
    let registry = registry();
    let session = registry.create(request()).await.expect("create");
    let mut attachment = session.attach(options(0)).expect("attach");
    registry.shutdown().await;
    assert_eq!(session.meta().status, SessionStatus::Stopped);
    attachment.recv().await.expect("stop event");
    assert_eq!(attachment.recv().await, Err(AttachError::Closed));
    assert!(matches!(
        registry.create(request()).await,
        Err(CreateError::Stopping)
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

#[tokio::test]
async fn command_catalog_survives_history_eviction_and_can_be_cleared() {
    let session = pending_session(Limits {
        history_capacity: NonZeroUsize::new(2).expect("nonzero"),
        ..Limits::default()
    });
    let commands = acp::AvailableCommandsUpdate::new(vec![acp::AvailableCommand::new(
        "review",
        "Review files",
    )]);
    session.apply(Update::Acp(acp::SessionUpdate::AvailableCommandsUpdate(
        commands.clone(),
    )));
    for index in 0..3 {
        session.set_title(&format!("{index}")).expect("rename");
    }
    assert!(
        session
            .page(None, 10)
            .expect("history")
            .events
            .iter()
            .all(|event| !matches!(
                &event.kind,
                EventKind::Acp(acp::SessionUpdate::AvailableCommandsUpdate(_))
            ))
    );
    assert_eq!(session.meta().available_commands, Some(commands));
    session.apply(Update::Acp(acp::SessionUpdate::AvailableCommandsUpdate(
        acp::AvailableCommandsUpdate::new(vec![]),
    )));
    assert!(
        session
            .meta()
            .available_commands
            .expect("catalog")
            .available_commands
            .is_empty()
    );
    session.shutdown().await;
}
