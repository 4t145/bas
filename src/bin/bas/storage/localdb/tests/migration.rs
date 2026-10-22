use super::*;
use crate::storage::{Memory, Storage};
use bas::{acp, model::SessionStatus};
use std::{io::Write, num::NonZeroUsize};

const SCHEMA_SIX: MigrationSet = MigrationSet::new(&[
    MigrationFile::new(
        1,
        "session_history",
        include_str!("../../migrations/0001_session_history.sql"),
    ),
    MigrationFile::new(
        2,
        "accepted_prompts",
        include_str!("../../migrations/0002_accepted_prompts.sql"),
    ),
    MigrationFile::new(
        3,
        "history_dependencies",
        include_str!("../../migrations/0003_history_dependencies.sql"),
    ),
    MigrationFile::new(
        4,
        "history_turns",
        include_str!("../../migrations/0004_history_turns.sql"),
    ),
    MigrationFile::new(
        5,
        "last_used_agent",
        include_str!("../../migrations/0005_last_used_agent.sql"),
    ),
    MigrationFile::new(
        6,
        "turn_blocks",
        include_str!("../../migrations/0006_turn_blocks.sql"),
    ),
]);

const SCHEMA_SEVEN: MigrationSet = MigrationSet::new(&[
    MigrationFile::new(
        1,
        "session_history",
        include_str!("../../migrations/0001_session_history.sql"),
    ),
    MigrationFile::new(
        2,
        "accepted_prompts",
        include_str!("../../migrations/0002_accepted_prompts.sql"),
    ),
    MigrationFile::new(
        3,
        "history_dependencies",
        include_str!("../../migrations/0003_history_dependencies.sql"),
    ),
    MigrationFile::new(
        4,
        "history_turns",
        include_str!("../../migrations/0004_history_turns.sql"),
    ),
    MigrationFile::new(
        5,
        "last_used_agent",
        include_str!("../../migrations/0005_last_used_agent.sql"),
    ),
    MigrationFile::new(
        6,
        "turn_blocks",
        include_str!("../../migrations/0006_turn_blocks.sql"),
    ),
    MigrationFile::new(
        7,
        "history_segments",
        include_str!("../../migrations/0007_history_segments.sql"),
    ),
]);

#[tokio::test]
async fn schema_eight_rebuilds_prompt_metadata_without_rewriting_packed_chunks() {
    let db = toasty::Db::builder()
        .models(toasty::models!(SessionRow, ChunkRow, AcceptanceRow))
        .max_pool_size(1)
        .build(toasty_driver_sqlite::Sqlite::in_memory())
        .await
        .expect("database");
    SCHEMA_SEVEN.apply(&db).await.expect("schema seven");
    let store = SqliteStore { db };
    let mut meta = SessionMeta::new(SessionId::new(), "legacy".into(), Owner::current());
    store.create(meta.clone(), None).await.expect("session");
    let turn = TurnId::new();
    let mut events = Vec::new();
    for kind in [
        EventKind::TurnStarted { turn },
        EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
            acp::ContentChunk::new("old prompt".into()),
        )),
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("answer ".repeat(1000).into()),
        )),
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        },
    ] {
        let event = Event::new(meta.id, meta.revision.next(), kind);
        meta.apply_event(&event);
        events.push(event);
    }
    let raw_events = serde_json::to_vec(&events).expect("raw events");
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw_events).expect("compress");
    let packed = encoder.finish().expect("packed payload");
    sql::statement("INSERT INTO history_chunks VALUES (?1, 1, 4, 1, 1, 1, ?2, ?3)")
        .bind(meta.id.to_string())
        .bind(raw_events.len() as i64)
        .bind(packed.clone())
        .exec(&mut store.db.clone())
        .await
        .expect("schema seven chunk");
    sql::statement(
        "INSERT INTO history_segments VALUES (?1, 1, 4, 1, 'turn', ?2, 4, 1, 1, 1, X'00')",
    )
    .bind(meta.id.to_string())
    .bind(turn.to_string())
    .exec(&mut store.db.clone())
    .await
    .expect("old derived segment");
    sql::statement("INSERT INTO history_state VALUES (?1, 4, X'00')")
        .bind(meta.id.to_string())
        .exec(&mut store.db.clone())
        .await
        .expect("old derived state");
    sql::statement("UPDATE sessions SET revision = 4, metadata = ?1 WHERE id = ?2")
        .bind(serde_json::to_vec(&meta).expect("meta"))
        .bind(meta.id.to_string())
        .exec(&mut store.db.clone())
        .await
        .expect("snapshot");
    verify_existing_schema(&store.db)
        .await
        .expect("valid seven");
    MIGRATIONS.apply(&store.db).await.expect("upgrade eight");
    verify_existing_schema(&store.db)
        .await
        .expect("valid eight");
    assert!(
        sql::query("SELECT cursor FROM history_state")
            .exec(&mut store.db.clone())
            .await
            .expect("invalidated state")
            .is_empty()
    );
    let overview = store
        .overview(meta.id)
        .await
        .expect("lazy metadata rebuild");
    assert_eq!(overview.entries.len(), 1);
    assert_eq!(overview.entries[0].cursor, 1);
    assert_eq!(overview.entries[0].preview, "old prompt");
    assert_eq!(
        overview.entries[0].timestamp_millis,
        events[0].at.millis() as i64
    );
    assert_eq!(
        store
            .window_at(meta.id, Cursor::from_raw(2))
            .await
            .expect("packed containing page")
            .window
            .events,
        events
    );
    let row = sql::query("SELECT payload FROM history_chunks")
        .exec(&mut store.db.clone())
        .await
        .expect("raw payload")
        .remove(0);
    assert_eq!(
        field::<Vec<u8>>(&mut columns(row).expect("row")).expect("bytes"),
        packed
    );
}

async fn legacy() -> SqliteStore {
    let db = toasty::Db::builder()
        .models(toasty::models!(SessionRow, EventRow, AcceptanceRow))
        .max_pool_size(1)
        .build(toasty_driver_sqlite::Sqlite::in_memory())
        .await
        .expect("legacy database");
    SCHEMA_SIX.apply(&db).await.expect("schema six");
    SqliteStore { db }
}

async fn raw_event(store: &SqliteStore, event: &Event) {
    sql::statement("INSERT INTO events(session_id, cursor, payload) VALUES (?1, ?2, ?3)")
        .bind(event.session.to_string())
        .bind(raw(event.cursor).expect("cursor"))
        .bind(serde_json::to_vec(event).expect("event"))
        .exec(&mut store.db.clone())
        .await
        .expect("raw event");
}

async fn block(store: &SqliteStore, turn: TurnId, events: &[Event], compress: bool) {
    let first = events.first().expect("nonempty");
    let last = events.last().expect("nonempty");
    let json = serde_json::to_vec(events).expect("array");
    let bytes = json.len();
    let payload = if compress {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&json).expect("compress");
        encoder.finish().expect("finish")
    } else {
        json
    };
    sql::statement("INSERT INTO history_blocks(session_id, first_cursor, last_cursor, turn_id, format, codec, raw_bytes, payload, floor) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?2)")
        .bind(first.session.to_string()).bind(raw(first.cursor).expect("first"))
        .bind(raw(last.cursor).expect("last")).bind(turn.to_string())
        .bind(i64::from(compress)).bind(bytes as i64).bind(payload)
        .exec(&mut store.db.clone()).await.expect("legacy block");
}

#[tokio::test]
async fn schema_six_mixed_history_migrates_losslessly_with_lazy_catalog() {
    let store = legacy().await;
    let memory = Memory::new(NonZeroUsize::new(100).expect("capacity"));
    let mut meta = SessionMeta::new(SessionId::new(), "legacy".into(), Owner::current());
    meta.status = SessionStatus::Idle;
    store
        .create(meta.clone(), Some(ResumeData::acp("native-id")))
        .await
        .expect("session");
    memory.create(meta.clone(), None).await.expect("reference");
    let turns = [TurnId::new(), TurnId::new()];
    let mut all = Vec::new();
    let first = Event::new(
        meta.id,
        meta.revision.next(),
        EventKind::TitleChanged {
            title: "initial".into(),
        },
    );
    meta.apply_event(&first);
    raw_event(&store, &first).await;
    all.push(first);
    for (index, turn) in turns.into_iter().enumerate() {
        let mut events = Vec::new();
        for kind in [
            EventKind::TurnStarted { turn },
            EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
                acp::ContentChunk::new("repeat ".repeat(1000).into()),
            )),
            EventKind::TurnFinished {
                turn,
                result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
            },
        ] {
            let event = Event::new(meta.id, meta.revision.next(), kind);
            meta.apply_event(&event);
            events.push(event);
        }
        if index == 0 {
            // v6 also splits a turn into multiple physical blocks.
            block(&store, turn, &events[..2], true).await;
            block(&store, turn, &events[2..], false).await;
        } else {
            for event in &events {
                raw_event(&store, event).await;
            }
        }
        all.extend(events);
    }
    memory
        .commit(meta.clone(), &all, None)
        .await
        .expect("reference history");
    sql::statement("UPDATE sessions SET revision = ?1, metadata = ?2 WHERE id = ?3")
        .bind(raw(meta.revision).expect("revision"))
        .bind(serde_json::to_vec(&meta).expect("meta"))
        .bind(meta.id.to_string())
        .exec(&mut store.db.clone())
        .await
        .expect("metadata");
    sql::statement("INSERT INTO accepted_prompts VALUES (?1, ?2, 2, ?3)")
        .bind(meta.id.to_string())
        .bind(turns[0].to_string())
        .bind(vec![9_u8; 32])
        .exec(&mut store.db.clone())
        .await
        .expect("receipt");
    verify_existing_schema(&store.db).await.expect("valid six");
    MIGRATIONS.apply(&store.db).await.expect("upgrade");
    verify_existing_schema(&store.db)
        .await
        .expect("valid seven");
    let rows = sql::query("SELECT name FROM sqlite_master WHERE type = 'table' AND (name LIKE 'history_%' OR name = 'events') ORDER BY name")
        .exec(&mut store.db.clone()).await.expect("table inventory");
    let names: Vec<String> = rows
        .into_iter()
        .map(|row| field(&mut columns(row).expect("row")).expect("name"))
        .collect();
    assert_eq!(
        names,
        ["history_chunks", "history_segments", "history_state"]
    );
    let rows = sql::query("SELECT first_cursor FROM history_segments")
        .exec(&mut store.db.clone())
        .await
        .expect("lazy segments");
    assert!(
        rows.is_empty(),
        "migration does not replay history or recompress blocks"
    );
    assert_eq!(
        store.page(meta.id, None, 100).await.expect("page").events,
        all
    );
    assert_eq!(store.tail(meta.id, 4).await.expect("tail"), all[3..]);
    assert_eq!(
        store.acceptance(meta.id, turns[0]).await.expect("receipt"),
        Some(PromptAcceptance {
            receipt: TurnReceipt {
                turn: turns[0],
                cursor: Cursor::from_raw(2)
            },
            fingerprint: [9; 32]
        })
    );
    for limit in [1, 2, 10] {
        assert_eq!(
            store
                .window(meta.id, None, limit)
                .await
                .expect("rebuilt catalog"),
            memory
                .window(meta.id, None, limit)
                .await
                .expect("reference window")
        );
    }
    store
        .pack_ready(meta.id, Cursor::START..=meta.revision)
        .await
        .expect("pack migrated chunks");
    assert_eq!(
        store
            .page(meta.id, None, 100)
            .await
            .expect("repacked")
            .events,
        all
    );
    MIGRATIONS
        .apply(&store.db)
        .await
        .expect("idempotent migration");
}

#[tokio::test]
async fn failed_migration_rolls_back_without_dropping_original_history() {
    let store = legacy().await;
    let meta = SessionMeta::new(SessionId::new(), "legacy".into(), Owner::current());
    store.create(meta.clone(), None).await.expect("session");
    let turn = TurnId::new();
    let event = Event::new(
        meta.id,
        Cursor::from_raw(1),
        EventKind::TurnStarted { turn },
    );
    raw_event(&store, &event).await;
    block(&store, turn, std::slice::from_ref(&event), false).await;
    // Deliberately overlapping v6 sources collide during the second payload copy.
    assert!(MIGRATIONS.apply(&store.db).await.is_err());
    verify_existing_schema(&store.db)
        .await
        .expect("original schema intact");
    let row = sql::query("PRAGMA user_version")
        .exec(&mut store.db.clone())
        .await
        .expect("version")
        .remove(0);
    assert_eq!(
        field::<i64>(&mut columns(row).expect("row")).expect("version"),
        6
    );
    for table in ["events", "history_blocks"] {
        let rows = sql::query(format!("SELECT payload FROM {table}"))
            .exec(&mut store.db.clone())
            .await
            .expect("original data");
        assert_eq!(rows.len(), 1);
    }
    let rows = sql::query("SELECT name FROM sqlite_master WHERE name = 'history_chunks'")
        .exec(&mut store.db.clone())
        .await
        .expect("rollback");
    assert!(
        rows.is_empty(),
        "failed migration leaves no partial destination"
    );
}
