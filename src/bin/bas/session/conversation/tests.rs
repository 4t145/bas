use super::*;
use bas::model::Owner;
use std::{num::NonZeroUsize, time::Duration};

#[tokio::test]
async fn only_sessions_without_storage_retain_a_ring() {
    // Both durable and bounded in-memory storage own their replay retention.
    for backend in ["none", "memory", "database"] {
        let storage: Option<Arc<dyn Storage>> = match backend {
            "none" => None,
            "memory" => Some(Arc::new(storage::Memory::new(
                NonZeroUsize::new(10).expect("capacity"),
            ))),
            _ => Some(Arc::new(
                storage::LocalDb::open(":memory:").await.expect("database"),
            )),
        };
        let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
        meta.status = SessionStatus::Idle;
        if let Some(storage) = &storage {
            storage.create(meta.clone(), None).await.expect("create");
        }
        let session = Session::with_storage(
            meta,
            Limits {
                history_capacity: NonZeroUsize::new(2).expect("capacity"),
                ..Limits::default()
            },
            None,
            storage.clone(),
            true,
        );
        assert_eq!(session.lock().history.is_some(), storage.is_none());
        for title in ["first", "second", "third"] {
            session.set_title(title).await.expect("rename");
        }
        assert_eq!(session.lock().history.is_some(), storage.is_none());
        let mut retained = Vec::new();
        session
            .try_visit_history(None, 10, |event, _| {
                retained.push(event.clone());
                true
            })
            .await
            .expect("history");
        assert_eq!(retained.len(), if storage.is_some() { 3 } else { 2 });
        if storage.is_none() {
            assert!(matches!(
                session.page(Some(Cursor::START), 10),
                Err(HistoryError::Expired { .. })
            ));
        }

        // Capture replay, then publish before draining it: cross the replay/live
        // boundary exactly once, for both cursor and tail attachments.
        let mut tail = session
            .try_attach(AttachOptions {
                cursor: None,
                replay: 2,
            })
            .await
            .expect("tail");
        let mut cursor = session
            .try_attach(AttachOptions {
                cursor: Some(Cursor::from_raw(1)),
                replay: 0,
            })
            .await
            .expect("cursor");
        session.set_title("fourth").await.expect("live rename");
        for attachment in [&mut tail, &mut cursor] {
            for (raw, title) in [(2, "second"), (3, "third"), (4, "fourth")] {
                let event = tokio::time::timeout(Duration::from_secs(5), attachment.recv())
                    .await
                    .expect("event arrives")
                    .expect("event");
                assert_eq!(event.cursor, Cursor::from_raw(raw));
                assert_eq!(
                    event.kind,
                    EventKind::TitleChanged {
                        title: title.into()
                    }
                );
            }
        }
        assert_eq!(session.lock().history.is_some(), storage.is_none());
    }
}
