//! Explicit, local-only profiling fixture; never reads a user's database.
use super::*;
use bas::{
    acp,
    model::{EventKind, Owner},
};
use std::time::Instant;

#[tokio::test]
#[ignore = "manual synthetic 20k-event storage profile"]
async fn synthetic_20k_history_profile() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("private/history.sqlite");
    let storage = LocalDb::open(path).await.expect("temporary database");
    let mut meta = SessionMeta::new(SessionId::new(), "fixture".into(), Owner::current());
    storage.create(meta.clone(), None).await.expect("create");
    let events: Vec<_> = (0..20_000)
        .map(|index| {
            let update: acp::SessionUpdate = serde_json::from_value(serde_json::json!({
                "sessionUpdate": if index % 10 == 0 { "user_message_chunk" } else { "agent_message_chunk" },
                "messageId": format!("message-{}", index / 10),
                "content": {"type": "text", "text": "A representative streamed text chunk."}
            }))
            .expect("message");
            let event = Event::new(meta.id, meta.revision.next(), EventKind::Acp(update));
            meta.apply_event(&event);
            event
        })
        .collect();
    let started = Instant::now();
    storage
        .commit(meta.clone(), &events, None)
        .await
        .expect("seed");
    let seed = started.elapsed();
    storage
        .test_sql(localdb::TestSql::ClearHistoryIndex)
        .await
        .expect("legacy index state");
    let started = Instant::now();
    let newest = storage.window(meta.id, None, 1).await.expect("first use");
    let first = started.elapsed();
    assert_eq!(newest.events.len(), 10);
    let started = Instant::now();
    let mut before = newest.before;
    for _ in 0..20 {
        before = storage
            .window(meta.id, before, 1)
            .await
            .expect("older indexed window")
            .before;
    }
    let older = started.elapsed();
    let started = Instant::now();
    let mut after = None;
    let mut count = 0;
    loop {
        let page = storage
            .page(meta.id, after, 256)
            .await
            .expect("legacy raw page");
        count += page.events.len();
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    let full = started.elapsed();
    assert_eq!(count, 20_000);
    let started = Instant::now();
    for index in 0..100 {
        let event = Event::new(
            meta.id,
            meta.revision.next(),
            EventKind::TitleChanged {
                title: format!("incremental {index}"),
            },
        );
        meta.apply_event(&event);
        storage
            .commit(meta.clone(), &[event], None)
            .await
            .expect("incremental commit");
    }
    let incremental = started.elapsed();
    eprintln!(
        "20k events/2k identities: seed={seed:?}, first-use newest={first:?}, \
         20 indexed older windows={older:?}, full raw replay={full:?}, \
         100 one-event commits={incremental:?}"
    );
}
