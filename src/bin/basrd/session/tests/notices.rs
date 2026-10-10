use std::sync::Arc;

use basr::{
    acp,
    model::{Cursor, EventKind, Owner, SessionId, SessionMeta},
};
use tokio::sync::mpsc;

use super::{AttachOptions, Limits, PendingAgent, Session, TIMEOUT};
use crate::{
    agent::Update,
    storage::{LocalDb, Storage},
};

#[tokio::test]
async fn live_notices_leave_memory_and_durable_history_and_cursors_unchanged() {
    for persistent in [false, true] {
        let storage: Option<Arc<dyn Storage>> = if persistent {
            Some(Arc::new(LocalDb::open(":memory:").await.expect("database")))
        } else {
            None
        };
        let meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
        if let Some(storage) = &storage {
            storage
                .create(meta.clone(), None)
                .await
                .expect("save session");
        }
        let limits = Limits::default();
        let session = Session::with_storage(meta, limits, None, storage.clone(), true);
        let (updates, inbox) = mpsc::channel(limits.live_capacity.get());
        session.listen(inbox);
        session
            .ready(Box::new(PendingAgent { _updates: updates }))
            .await
            .expect("ready");
        let before = session.meta();
        let mut live = session
            .try_attach(AttachOptions {
                replay: 0,
                cursor: Some(before.revision),
            })
            .await
            .expect("live attachment");

        for severity in [acp::NoticeSeverity::Info, acp::NoticeSeverity::Warning] {
            let notice = acp::Notice::new(severity, "Provider status")
                .description("Retrying the upstream request");
            session
                .apply(Update::Acp(acp::SessionUpdate::Notice(notice.clone())))
                .await;
            let received = tokio::time::timeout(TIMEOUT, live.recv())
                .await
                .expect("notice arrives")
                .expect("live notice");
            assert!(received.is_transient());
            assert_eq!(received.cursor, Cursor::START);
            assert_eq!(
                received.kind,
                EventKind::Acp(acp::SessionUpdate::Notice(notice))
            );
            assert_eq!(live.cursor(), before.revision);
            assert_eq!(session.meta(), before);
        }
        assert!(
            session
                .page(Some(before.revision), 10)
                .expect("history")
                .events
                .is_empty()
        );
        if let Some(storage) = &storage {
            assert_eq!(
                storage
                    .load(session.id())
                    .await
                    .expect("load")
                    .expect("saved")
                    .meta,
                before
            );
            assert!(
                storage
                    .page(session.id(), Some(before.revision), 10)
                    .await
                    .expect("persisted history")
                    .events
                    .is_empty()
            );
        }

        for title in ["first", "second"] {
            session.set_title(title).await.expect("rename");
        }
        let history = session
            .page(Some(before.revision), 10)
            .expect("history")
            .events;
        assert_eq!(history.len(), 2);
        for (index, expected) in history.iter().enumerate() {
            assert!(!expected.is_transient());
            assert_eq!(
                expected.cursor.raw(),
                before.revision.raw() + index as u64 + 1
            );
            assert_eq!(live.recv().await.expect("ordered event"), *expected);
        }
        if let Some(storage) = &storage {
            assert_eq!(
                storage
                    .page(session.id(), Some(before.revision), 10)
                    .await
                    .expect("persisted history")
                    .events,
                history
            );
        }
        let mut replay = session
            .try_attach(AttachOptions {
                replay: 100,
                cursor: Some(before.revision),
            })
            .await
            .expect("replay attachment");
        for expected in history {
            assert_eq!(replay.recv().await.expect("replayed event"), expected);
        }
        session.shutdown().await;
    }
}
