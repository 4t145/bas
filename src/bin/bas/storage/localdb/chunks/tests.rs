use super::*;
use crate::storage::{Memory, Storage};
use bas::{
    acp,
    model::{EventKind, Owner, SessionStatus},
};
use std::num::NonZeroUsize;

#[path = "tests/navigation.rs"]
mod navigation;
#[path = "tests/segments.rs"]
mod segments;

fn meta() -> SessionMeta {
    SessionMeta::new(SessionId::new(), "fixture".into(), Owner::current())
}

fn directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("private database");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private directory permissions");
    }
    directory
}

fn chunk(text: impl Into<String>) -> EventKind {
    EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
        acp::ContentChunk::new(text.into().into()),
    ))
}

fn finish(turn: TurnId) -> EventKind {
    EventKind::TurnFinished {
        turn,
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
    }
}

fn batch(meta: &mut SessionMeta, kinds: impl IntoIterator<Item = EventKind>) -> Vec<Event> {
    kinds
        .into_iter()
        .map(|kind| {
            let event = Event::new(meta.id, meta.revision.next(), kind);
            meta.apply_event(&event);
            event
        })
        .collect()
}

async fn count(store: &SqliteStore, table: &str) -> i64 {
    let row = sql::query(format!("SELECT COUNT(*) FROM {table}"))
        .exec(&mut store.db.clone())
        .await
        .expect("count")
        .remove(0);
    field(&mut columns(row).expect("row")).expect("count field")
}

impl SqliteStore {
    async fn pack_turn(&self, id: SessionId, turn: TurnId, terminal: Cursor) -> Result<(), Error> {
        let row = sql::query("SELECT first_cursor FROM history_segments WHERE session_id = ?1 AND turn_id = ?2 AND terminal_cursor = ?3")
            .bind(id.to_string()).bind(turn.to_string()).bind(raw(terminal)?)
            .exec(&mut self.db.clone()).await?.into_iter().next()
            .ok_or(Error::Corrupt("missing completed test turn"))?;
        self.pack_segment(id, revision(field(&mut columns(row)?)?)?)
            .await
    }
}

#[tokio::test]
async fn completed_turn_packs_payloads_and_indexes_without_changing_any_reader() {
    let directory = directory();
    let path = directory.path().join("sessions.sqlite3");
    let store = SqliteStore::open(&path).await.expect("open");
    let mut meta = meta();
    store
        .create(meta.clone(), Some(ResumeData::acp("original")))
        .await
        .expect("create");
    let mut expected = batch(
        &mut meta,
        [EventKind::TitleChanged {
            title: "outside turn".into(),
        }],
    );
    store
        .commit(meta.clone(), &expected, None)
        .await
        .expect("metadata event");
    let turn = TurnId::new();
    let start = batch(&mut meta, [EventKind::TurnStarted { turn }]);
    let accepted = PromptAcceptance {
        receipt: TurnReceipt {
            turn,
            cursor: start[0].cursor,
        },
        fingerprint: [7; 32],
    };
    store
        .commit(meta.clone(), &start, Some(accepted))
        .await
        .expect("accept");
    expected.extend(start);
    let deltas = batch(&mut meta, (0..1000).map(|_| chunk("streaming text ")));
    store
        .commit(meta.clone(), &deltas, None)
        .await
        .expect("active deltas");
    expected.extend(deltas);
    assert_eq!(count(&store, "history_chunks").await, 1002);
    assert_eq!(count(&store, "history_segments").await, 2);
    let done = batch(&mut meta, [finish(turn)]);
    store
        .commit(meta.clone(), &done, None)
        .await
        .expect("finish");
    expected.extend(done);
    let window = store
        .window(meta.id, None, 1)
        .await
        .expect("original window");
    let older = store
        .window(meta.id, window.before, 1)
        .await
        .expect("original prefix");
    store
        .pack_turn(meta.id, turn, meta.revision)
        .await
        .expect("pack");
    assert_eq!(
        count(&store, "history_chunks").await,
        2,
        "initialization and completed turn use the same chunk table"
    );
    assert_eq!(count(&store, "history_segments").await, 2);
    assert_eq!(count(&store, "history_state").await, 1);
    let mut db = store.db.clone();
    let mut tx = db.transaction().await.expect("scan snapshot");
    let mut batch = rows(
        &mut tx,
        meta.id,
        Cursor::from_raw(2)..=meta.revision,
        usize::MAX,
    )
    .await
    .expect("one chunk scan");
    assert_eq!(batch.len(), 1);
    assert_eq!(
        decode(&mut tx, meta.id, meta.revision, batch.remove(0))
            .await
            .expect("dense chunk")
            .events
            .len(),
        1002,
        "reindex consumes a dense chunk once, not in 256-event slices"
    );
    tx.commit().await.expect("scan");
    let row = sql::query(
        "SELECT codec, raw_bytes, length(payload) FROM history_chunks WHERE first_cursor = 2",
    )
    .exec(&mut store.db.clone())
    .await
    .expect("encoding")
    .remove(0);
    let mut row = columns(row).expect("row");
    assert_eq!(field::<i64>(&mut row).expect("codec"), 1);
    let raw_bytes: i64 = field(&mut row).expect("raw bytes");
    let compressed_bytes: i64 = field(&mut row).expect("compressed bytes");
    assert!(compressed_bytes < raw_bytes);
    eprintln!(
        "1000-delta turn: 1002 event rows -> 1 block; payload {raw_bytes} -> {compressed_bytes} bytes"
    );
    for after in [0, 1, 2, 19, 999, 1003] {
        for limit in [1, 17, usize::MAX] {
            let page = store
                .page(meta.id, Some(Cursor::from_raw(after)), limit)
                .await
                .expect("page");
            let suffix: Vec<_> = expected
                .iter()
                .filter(|e| e.cursor.raw() > after)
                .cloned()
                .collect();
            assert_eq!(
                page.events,
                suffix.iter().take(limit).cloned().collect::<Vec<_>>()
            );
            assert_eq!(
                page.next,
                (suffix.len() > limit).then(|| page.events.last().expect("nonempty").cursor)
            );
        }
    }
    for count in [0, 1, 7, 2000] {
        assert_eq!(
            store.tail(meta.id, count).await.expect("tail"),
            expected[expected.len().saturating_sub(count)..]
        );
    }
    assert_eq!(
        store.acceptance(meta.id, turn).await.expect("receipt"),
        Some(accepted)
    );
    assert_eq!(
        store.window(meta.id, None, 1).await.expect("packed window"),
        window
    );
    assert_eq!(
        store
            .window(meta.id, window.before, 1)
            .await
            .expect("packed prefix"),
        older
    );
    store
        .pack_turn(meta.id, turn, meta.revision)
        .await
        .expect("idempotent packing");
    drop(store);
    let reopened = SqliteStore::open(&path).await.expect("reopen");
    assert_eq!(
        reopened
            .page(meta.id, None, usize::MAX)
            .await
            .expect("history")
            .events,
        expected
    );
    assert_eq!(
        reopened
            .acceptance(meta.id, turn)
            .await
            .expect("durable receipt"),
        Some(accepted)
    );
    assert!(reopened.delete(meta.id).await.expect("delete"));
    for table in [
        "history_chunks",
        "history_segments",
        "history_state",
        "accepted_prompts",
    ] {
        assert_eq!(count(&reopened, table).await, 0);
    }
}

#[tokio::test]
async fn worker_packs_after_commit_and_a_failed_pack_does_not_fail_the_turn() {
    let directory = directory();
    let path = directory.path().join("sessions.sqlite3");
    let store = LocalDb::open(&path).await.expect("open worker");
    let mut meta = meta();
    store.create(meta.clone(), None).await.expect("create");
    let writer = rusqlite::Connection::open(&path).expect("test connection");
    writer.execute_batch("CREATE TRIGGER fail_pack BEFORE INSERT ON history_chunks WHEN NEW.last_cursor > NEW.first_cursor BEGIN SELECT RAISE(ABORT, 'packing unavailable'); END;")
        .expect("inject maintenance failure");
    let turn = TurnId::new();
    let first = batch(
        &mut meta,
        [
            EventKind::TurnStarted { turn },
            chunk("kept raw"),
            finish(turn),
        ],
    );
    store
        .commit(meta.clone(), &first, None)
        .await
        .expect("successful commit despite failed maintenance");
    assert_eq!(
        store
            .page(meta.id, None, 10)
            .await
            .expect("read barrier")
            .events,
        first
    );
    assert_eq!(
        writer
            .query_row(
                "SELECT COUNT(*) FROM history_chunks WHERE last_cursor > first_cursor",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("count"),
        0
    );
    assert_eq!(
        writer
            .query_row("SELECT COUNT(*) FROM history_chunks", [], |r| r
                .get::<_, i64>(0))
            .expect("small chunks preserved"),
        3
    );
    writer
        .execute_batch("DROP TRIGGER fail_pack")
        .expect("restore packing");
    let turn = TurnId::new();
    let second = batch(
        &mut meta,
        [
            EventKind::TurnStarted { turn },
            chunk("packed"),
            finish(turn),
        ],
    );
    store
        .commit(meta.clone(), &second, None)
        .await
        .expect("next turn");
    let mut expected = first;
    expected.extend(second);
    assert_eq!(
        store
            .page(meta.id, None, 10)
            .await
            .expect("mixed history")
            .events,
        expected
    );
    assert_eq!(
        writer
            .query_row(
                "SELECT COUNT(*) FROM history_chunks WHERE last_cursor > first_cursor",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("auto packed"),
        2
    );
    assert_eq!(
        writer
            .query_row("SELECT COUNT(*) FROM history_segments", [], |r| r
                .get::<_, i64>(0))
            .expect("one catalog row per turn"),
        2
    );
}

#[tokio::test]
async fn bounded_blocks_interleave_with_oversized_raw_events() {
    let store = SqliteStore::open(":memory:").await.expect("database");
    let mut meta = meta();
    store.create(meta.clone(), None).await.expect("create");
    let turn = TurnId::new();
    let events = batch(
        &mut meta,
        [
            EventKind::TurnStarted { turn },
            chunk("a".repeat(MAX_BLOCK_BYTES / 2)),
            chunk("b".repeat(MAX_BLOCK_BYTES + 1)),
            chunk("c".repeat(MAX_BLOCK_BYTES / 2)),
            EventKind::TurnInterrupted {
                turn,
                reason: bas::model::InterruptionReason::DaemonRestarted,
            },
        ],
    );
    store
        .commit(meta.clone(), &events, None)
        .await
        .expect("interrupted turn");
    store
        .pack_turn(meta.id, turn, meta.revision)
        .await
        .expect("pack segments");
    assert_eq!(count(&store, "history_chunks").await, 3);
    assert_eq!(count(&store, "history_segments").await, 1);
    assert_eq!(
        store
            .page(meta.id, None, 10)
            .await
            .expect("all segments")
            .events,
        events
    );
    assert_eq!(
        store
            .page(meta.id, Some(events[1].cursor), 2)
            .await
            .expect("mixed page")
            .events,
        events[2..4]
    );
    assert_eq!(store.tail(meta.id, 2).await.expect("tail"), events[3..]);
}

fn update(value: serde_json::Value) -> EventKind {
    EventKind::Acp(serde_json::from_value(value).expect("ACP update"))
}

#[tokio::test]
async fn packed_context_and_cross_turn_dependencies_match_memory_even_after_reindex() {
    use serde_json::json;
    let store = SqliteStore::open(":memory:").await.expect("database");
    let memory = Memory::new(NonZeroUsize::new(100).expect("capacity"));
    let mut meta = meta();
    meta.status = SessionStatus::Idle;
    store.create(meta.clone(), None).await.expect("create");
    memory.create(meta.clone(), None).await.expect("memory");
    let mut all = Vec::new();
    for index in 0..6 {
        let turn = TurnId::new();
        let mut kinds = vec![
            EventKind::TurnStarted { turn },
            chunk(format!("turn {index}")),
        ];
        if index == 0 {
            kinds.extend([
                update(json!({"sessionUpdate":"plan_update","plan":{"type":"markdown","planId":"p","content":"original"}})),
                update(json!({"sessionUpdate":"usage_update","used":5,"size":100})),
                update(json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"read","status":"in_progress"})),
            ]);
        }
        if index == 3 {
            kinds.push(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed","content":[]})));
        }
        if index == 1 {
            kinds.push(update(json!({"sessionUpdate":"plan_update","plan":{"type":"markdown","planId":"p","content":"changed"}})));
        }
        if index == 4 {
            kinds.push(update(json!({"sessionUpdate":"plan_removed","planId":"p"})));
            kinds.push(update(
                json!({"sessionUpdate":"usage_update","used":10,"size":100}),
            ));
        }
        kinds.push(finish(turn));
        let events = batch(&mut meta, kinds);
        store
            .commit(meta.clone(), &events, None)
            .await
            .expect("SQLite commit");
        memory
            .commit(meta.clone(), &events, None)
            .await
            .expect("memory commit");
        all.extend(events);
        store
            .pack_turn(meta.id, turn, meta.revision)
            .await
            .expect("pack");
        assert_eq!(
            store.window(meta.id, None, 1).await.expect("SQLite window"),
            memory
                .window(meta.id, None, 1)
                .await
                .expect("memory window")
        );
        if index == 2 {
            assert_eq!(
                store
                    .window(meta.id, None, 1)
                    .await
                    .expect("active context")
                    .context
                    .len(),
                3,
                "original plan, latest archived plan update and usage"
            );
        }
        if index == 5 {
            assert_eq!(
                store
                    .window(meta.id, None, 1)
                    .await
                    .expect("removed plan")
                    .context
                    .len(),
                1,
                "only the latest usage survives the archived plan tombstone"
            );
        }
    }
    assert_eq!(count(&store, "history_chunks").await, 6);
    assert_eq!(count(&store, "history_segments").await, 6);
    for table in ["history_state", "history_segments"] {
        sql::statement(format!("DELETE FROM {table}"))
            .exec(&mut store.db.clone())
            .await
            .expect("clear derived index");
    }
    assert_eq!(
        store
            .window(meta.id, None, 1)
            .await
            .expect("rebuilt window"),
        memory
            .window(meta.id, None, 1)
            .await
            .expect("reference window")
    );
    assert_eq!(
        count(&store, "history_segments").await,
        6,
        "rebuild retains one catalog row per turn"
    );
    assert_eq!(count(&store, "history_state").await, 1);
    assert_eq!(
        store
            .page(meta.id, None, 100)
            .await
            .expect("unchanged payloads")
            .events,
        all
    );
}

#[test]
fn codec_roundtrips_and_rejects_corruption_without_unbounded_inflation() {
    let mut meta = meta();
    let events = batch(&mut meta, (0..100).map(|_| chunk("repeatable content")));
    let encoded = codec::encode(&events).expect("compress");
    assert_eq!(encoded.codec, 1);
    assert_eq!(
        codec::decode(encoded.codec, encoded.raw_bytes as i64, &encoded.payload).expect("decode"),
        events
    );
    let json = serde_json::to_vec(&events).expect("JSON");
    assert_eq!(
        codec::decode(0, json.len() as i64, &json).expect("plain"),
        events
    );
    assert!(codec::decode(99, json.len() as i64, &json).is_err());
    assert!(codec::decode(1, (MAX_BLOCK_BYTES + 1) as i64, &encoded.payload).is_err());
    assert!(codec::decode(1, 1, &encoded.payload).is_err());
    assert!(
        codec::decode(
            1,
            encoded.raw_bytes as i64,
            &encoded.payload[..encoded.payload.len() - 1]
        )
        .is_err()
    );
    let mut trailing = encoded.payload;
    trailing.push(0);
    assert!(codec::decode(1, encoded.raw_bytes as i64, &trailing).is_err());
}

#[tokio::test]
async fn readers_reject_overlaps_but_do_not_decode_unrequested_blocks() {
    let store = SqliteStore::open(":memory:").await.expect("database");
    let mut meta = meta();
    store.create(meta.clone(), None).await.expect("create");
    let turn = TurnId::new();
    let first = batch(
        &mut meta,
        [
            EventKind::TurnStarted { turn },
            chunk("first"),
            finish(turn),
        ],
    );
    store
        .commit(meta.clone(), &first, None)
        .await
        .expect("first");
    store
        .pack_turn(meta.id, turn, meta.revision)
        .await
        .expect("pack first");
    let payload = serde_json::to_vec(&first[1..2]).expect("event array");
    sql::statement("INSERT INTO history_chunks(session_id, first_cursor, last_cursor, format, codec, raw_bytes, payload) VALUES (?1, ?2, ?2, 1, 0, ?3, ?4)")
        .bind(meta.id.to_string())
        .bind(raw(first[1].cursor).expect("cursor"))
        .bind(payload.len() as i64)
        .bind(payload)
        .exec(&mut store.db.clone())
        .await
        .expect("overlap");
    assert!(matches!(
        store.page(meta.id, None, 10).await,
        Err(Error::Corrupt("overlapping history chunks"))
    ));
    sql::statement("DELETE FROM history_chunks WHERE first_cursor = 2")
        .exec(&mut store.db.clone())
        .await
        .expect("remove overlap");
    sql::statement("INSERT INTO history_chunks SELECT session_id, first_cursor + 1, last_cursor, segment_start, format, codec, raw_bytes, payload FROM history_chunks")
        .exec(&mut store.db.clone()).await.expect("overlapping block");
    assert!(matches!(
        store.page(meta.id, None, 10).await,
        Err(Error::Corrupt("overlapping history chunks"))
    ));
    sql::statement("DELETE FROM history_chunks WHERE first_cursor = 2")
        .exec(&mut store.db.clone())
        .await
        .expect("remove overlapping block");
    let turn = TurnId::new();
    let second = batch(
        &mut meta,
        [
            EventKind::TurnStarted { turn },
            chunk("second"),
            finish(turn),
        ],
    );
    store
        .commit(meta.clone(), &second, None)
        .await
        .expect("second");
    store
        .pack_turn(meta.id, turn, meta.revision)
        .await
        .expect("pack second");
    sql::statement("UPDATE history_chunks SET payload = X'00' WHERE first_cursor = 1")
        .exec(&mut store.db.clone())
        .await
        .expect("damage unrelated old payload");
    assert_eq!(
        store
            .window(meta.id, None, 1)
            .await
            .expect("recent window")
            .events,
        second
    );
    assert!(
        store.page(meta.id, None, 10).await.is_err(),
        "requested corruption is not skipped"
    );
}

#[tokio::test]
async fn schema_seven_requires_exact_history_table_definitions() {
    let store = SqliteStore::open(":memory:").await.expect("database");
    verify_existing_schema(&store.db)
        .await
        .expect("schema seven");
    sql::statement("DROP TABLE history_chunks")
        .exec(&mut store.db.clone())
        .await
        .expect("remove required table");
    assert!(matches!(
        verify_existing_schema(&store.db).await,
        Err(Error::Corrupt(_))
    ));
    let store = SqliteStore::open(":memory:").await.expect("database");
    sql::statement("ALTER TABLE history_segments ADD COLUMN unexpected TEXT")
        .exec(&mut store.db.clone())
        .await
        .expect("incompatible schema");
    assert!(matches!(
        verify_existing_schema(&store.db).await,
        Err(Error::Corrupt(_))
    ));
    let store = SqliteStore::open(":memory:").await.expect("database");
    sql::statement("DROP TABLE history_state")
        .exec(&mut store.db.clone())
        .await
        .expect("replace table");
    sql::statement("CREATE TABLE history_state (session_id TEXT NOT NULL PRIMARY KEY, cursor INTEGER NOT NULL, state TEXT)")
        .exec(&mut store.db.clone()).await.expect("wrong payload type and nullability");
    assert!(matches!(
        verify_existing_schema(&store.db).await,
        Err(Error::Corrupt(_))
    ));
}
