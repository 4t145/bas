use super::*;
use bas::model::{EventKind, Owner};

#[tokio::test]
async fn schema_four_upgrade_starts_without_selection_and_checks_version() {
    let mut db = toasty::Db::builder()
        .models(toasty::models!(SessionRow, EventRow, AcceptanceRow))
        .max_pool_size(1)
        .build(toasty_driver_sqlite::Sqlite::in_memory())
        .await
        .expect("database");
    const SCHEMA_FOUR: MigrationSet = MigrationSet::new(&[
        MigrationFile::new(
            1,
            "session_history",
            include_str!("../migrations/0001_session_history.sql"),
        ),
        MigrationFile::new(
            2,
            "accepted_prompts",
            include_str!("../migrations/0002_accepted_prompts.sql"),
        ),
        MigrationFile::new(
            3,
            "history_dependencies",
            include_str!("../migrations/0003_history_dependencies.sql"),
        ),
        MigrationFile::new(
            4,
            "history_turns",
            include_str!("../migrations/0004_history_turns.sql"),
        ),
    ]);
    SCHEMA_FOUR.apply(&db).await.expect("schema four");
    let store = SqliteStore { db: db.clone() };
    let mut legacy = SessionMeta::new(SessionId::new(), "legacy".into(), Owner::current());
    legacy.status = bas::model::SessionStatus::Idle;
    store
        .create(legacy.clone(), Some(ResumeData::acp("legacy")))
        .await
        .expect("legacy session");
    verify_existing_schema(&db).await.expect("valid four");
    MIGRATIONS.apply(&db).await.expect("apply new schema");
    assert_eq!(
        store
            .load(legacy.id)
            .await
            .expect("load")
            .expect("session")
            .meta,
        legacy
    );
    assert_eq!(
        store
            .last_used_agent()
            .await
            .expect("no inferred selection"),
        None
    );
    verify_existing_schema(&db).await.expect("valid five");
    sql::statement("PRAGMA user_version = 6")
        .exec(&mut db)
        .await
        .expect("future");
    assert!(matches!(
        verify_existing_schema(&db).await,
        Err(Error::InvalidCommit(_))
    ));
    sql::statement("PRAGMA user_version = 5")
        .exec(&mut db)
        .await
        .expect("restore");
    sql::statement("DROP TABLE agent_selection")
        .exec(&mut db)
        .await
        .expect("incomplete");
    assert!(matches!(
        verify_existing_schema(&db).await,
        Err(Error::Corrupt(_))
    ));
}

#[tokio::test]
async fn schema_three_upgrade_preserves_authoritative_records_and_rebuilds_turns() {
    let db = toasty::Db::builder()
        .models(toasty::models!(SessionRow, EventRow, AcceptanceRow))
        .max_pool_size(1)
        .build(toasty_driver_sqlite::Sqlite::in_memory())
        .await
        .expect("database");
    const SCHEMA_THREE: MigrationSet = MigrationSet::new(&[
        MigrationFile::new(
            1,
            "session_history",
            include_str!("../migrations/0001_session_history.sql"),
        ),
        MigrationFile::new(
            2,
            "accepted_prompts",
            include_str!("../migrations/0002_accepted_prompts.sql"),
        ),
        MigrationFile::new(
            3,
            "history_dependencies",
            include_str!("../migrations/0003_history_dependencies.sql"),
        ),
    ]);
    SCHEMA_THREE.apply(&db).await.expect("schema three");
    let store = SqliteStore { db };
    let mut meta = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    let resume = ResumeData::acp("original-resume");
    store
        .create(meta.clone(), Some(resume.clone()))
        .await
        .expect("session");
    let turn = TurnId::new();
    let events: Vec<_> = [
        EventKind::TitleChanged {
            title: "initial".into(),
        },
        EventKind::TurnStarted { turn },
        EventKind::TitleChanged {
            title: "turn activity".into(),
        },
    ]
    .into_iter()
    .map(|kind| {
        let event = Event::new(meta.id, meta.revision.next(), kind);
        meta.apply_event(&event);
        event
    })
    .collect();
    let mut db = store.db.clone();
    for event in &events {
        sql::statement("INSERT INTO events(session_id, cursor, payload) VALUES (?1, ?2, ?3)")
            .bind(meta.id.to_string())
            .bind(raw(event.cursor).expect("cursor"))
            .bind(serde_json::to_vec(event).expect("event"))
            .exec(&mut db)
            .await
            .expect("legacy event");
    }
    sql::statement("UPDATE sessions SET revision = ?1, metadata = ?2 WHERE id = ?3")
        .bind(raw(meta.revision).expect("revision"))
        .bind(serde_json::to_vec(&meta).expect("metadata"))
        .bind(meta.id.to_string())
        .exec(&mut db)
        .await
        .expect("legacy metadata");
    sql::statement("INSERT INTO history_index(session_id, cursor, state) VALUES (?1, ?2, ?3)")
        .bind(meta.id.to_string())
        .bind(raw(meta.revision).expect("revision"))
        .bind(b"old derived state".to_vec())
        .exec(&mut db)
        .await
        .expect("old index");
    sql::statement("INSERT INTO history_dependencies(session_id, cursor, floor, context_key, context_start) VALUES (?1, 1, 0, 'usage', 1)")
        .bind(meta.id.to_string()).exec(&mut db).await.expect("old dependencies");
    sql::statement(
        "INSERT INTO history_context_keys(session_id, context_key) VALUES (?1, 'usage')",
    )
    .bind(meta.id.to_string())
    .exec(&mut db)
    .await
    .expect("old context keys");
    sql::statement("INSERT INTO accepted_prompts(session_id, turn_id, cursor, fingerprint) VALUES (?1, ?2, 2, ?3)")
        .bind(meta.id.to_string()).bind(turn.to_string()).bind(vec![7u8; 32])
        .exec(&mut db).await.expect("acceptance");
    verify_existing_schema(&store.db)
        .await
        .expect("valid schema three");
    MIGRATIONS.apply(&store.db).await.expect("upgrade");
    let stored = store.load(meta.id).await.expect("load").expect("session");
    assert_eq!(stored.meta, meta);
    assert_eq!(stored.resume, Some(resume));
    assert_eq!(
        store.acceptance(meta.id, turn).await.expect("acceptance"),
        Some(PromptAcceptance {
            receipt: TurnReceipt {
                turn,
                cursor: Cursor::from_raw(2)
            },
            fingerprint: [7; 32],
        })
    );
    assert_eq!(
        store
            .page(meta.id, None, 10)
            .await
            .expect("raw history")
            .events,
        events
    );
    let recent = store.window(meta.id, None, 1).await.expect("rebuilt turns");
    assert_eq!(recent.events, events[1..]);
    assert_eq!(recent.meta, meta);
    assert_eq!(recent.cursor, meta.revision);
    let older = store
        .window(meta.id, recent.before, 1)
        .await
        .expect("initialization");
    assert_eq!(older.events, events[..1]);
    assert_eq!(older.before, None);
    verify_existing_schema(&store.db)
        .await
        .expect("valid schema five");
    assert_eq!(
        store.last_used_agent().await.expect("no history inference"),
        None
    );
    MIGRATIONS
        .apply(&store.db)
        .await
        .expect("idempotent upgrade");
}
