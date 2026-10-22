use super::*;

#[tokio::test]
async fn unrelated_terminal_does_not_seal_or_pack_the_active_turn() {
    for interrupted in [false, true] {
        let store = SqliteStore::open(":memory:").await.expect("database");
        let mut meta = meta();
        store.create(meta.clone(), None).await.expect("session");
        let turn = TurnId::new();
        let unrelated = TurnId::new();
        let terminal = if interrupted {
            EventKind::TurnInterrupted {
                turn: unrelated,
                reason: bas::model::InterruptionReason::DaemonRestarted,
            }
        } else {
            finish(unrelated)
        };
        let mut expected = batch(
            &mut meta,
            [
                EventKind::TurnStarted { turn },
                chunk("still active"),
                terminal,
            ],
        );
        store
            .commit(meta.clone(), &expected, None)
            .await
            .expect("events");
        store
            .pack_ready(meta.id, Cursor::START..=meta.revision)
            .await
            .expect("maintenance");
        let row = sql::query(
            "SELECT sealed, terminal_cursor FROM history_segments WHERE first_cursor = 1",
        )
        .exec(&mut store.db.clone())
        .await
        .expect("segment")
        .remove(0);
        let mut fields = columns(row).expect("row");
        assert_eq!(field::<i64>(&mut fields).expect("sealed"), 0);
        assert!(matches!(fields.first(), Some(Value::Null)));
        assert_eq!(
            count(&store, "history_chunks").await,
            3,
            "active chunks remain unpacked"
        );
        let rest = batch(&mut meta, [chunk("continuing"), finish(turn)]);
        store
            .commit(meta.clone(), &rest, None)
            .await
            .expect("matching completion");
        expected.extend(rest);
        store
            .pack_ready(meta.id, Cursor::START..=meta.revision)
            .await
            .expect("completed maintenance");
        assert_eq!(count(&store, "history_segments").await, 1);
        assert_eq!(count(&store, "history_chunks").await, 1);
        assert_eq!(
            store
                .page(meta.id, None, 10)
                .await
                .expect("all events retained")
                .events,
            expected
        );
        sql::statement("DELETE FROM history_state")
            .exec(&mut store.db.clone())
            .await
            .expect("rebuild");
        assert_eq!(
            store
                .window(meta.id, None, 1)
                .await
                .expect("reindexed chunk")
                .events,
            expected
        );
    }
}

#[tokio::test]
async fn late_control_updates_do_not_reopen_packed_turns_or_add_page_boundaries() {
    let store = SqliteStore::open(":memory:").await.expect("database");
    let memory = Memory::new(NonZeroUsize::new(100).expect("capacity"));
    let mut meta = meta();
    store.create(meta.clone(), None).await.expect("session");
    memory.create(meta.clone(), None).await.expect("reference");
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
        .expect("turn");
    memory
        .commit(meta.clone(), &first, None)
        .await
        .expect("reference turn");
    store
        .pack_turn(meta.id, turn, meta.revision)
        .await
        .expect("pack");
    let original = sql::query("SELECT payload FROM history_chunks WHERE first_cursor = 1")
        .exec(&mut store.db.clone())
        .await
        .expect("packed payload");
    let late = batch(
        &mut meta,
        [EventKind::TitleChanged {
            title: "late title".into(),
        }],
    );
    store
        .commit(meta.clone(), &late, None)
        .await
        .expect("control");
    memory
        .commit(meta.clone(), &late, None)
        .await
        .expect("reference control");
    assert_eq!(
        store.window(meta.id, None, 1).await.expect("window"),
        memory
            .window(meta.id, None, 1)
            .await
            .expect("reference window")
    );
    assert!(matches!(
        store.window(meta.id, Some(late[0].cursor), 1).await,
        Err(Error::NotTurnBoundary(_))
    ));
    let row = sql::query("SELECT kind, page_start FROM history_segments WHERE first_cursor = 4")
        .exec(&mut store.db.clone())
        .await
        .expect("control catalog")
        .remove(0);
    let mut fields = columns(row).expect("row");
    assert_eq!(field::<String>(&mut fields).expect("kind"), "control");
    assert_eq!(field::<i64>(&mut fields).expect("page"), 1);
    let next = TurnId::new();
    let second = batch(
        &mut meta,
        [
            EventKind::TurnStarted { turn: next },
            chunk("second"),
            finish(next),
        ],
    );
    store
        .commit(meta.clone(), &second, None)
        .await
        .expect("second");
    memory
        .commit(meta.clone(), &second, None)
        .await
        .expect("reference second");
    store
        .pack_ready(meta.id, Cursor::START..=meta.revision)
        .await
        .expect("seal control");
    let recent = store.window(meta.id, None, 1).await.expect("latest");
    assert_eq!(recent.events, second);
    assert_eq!(
        store
            .window(meta.id, recent.before, 1)
            .await
            .expect("older"),
        memory
            .window(meta.id, recent.before, 1)
            .await
            .expect("reference older")
    );
    assert_eq!(
        sql::query("SELECT payload FROM history_chunks WHERE first_cursor = 1")
            .exec(&mut store.db.clone())
            .await
            .expect("immutable packed turn"),
        original
    );
    assert_eq!(count(&store, "history_segments").await, 3);
    assert!(store.delete(meta.id).await.expect("delete"));
    let mut recreated = super::meta();
    recreated.id = meta.id;
    store
        .create(recreated.clone(), None)
        .await
        .expect("reuse id");
    assert!(
        store
            .page(meta.id, None, 10)
            .await
            .expect("empty recreation")
            .events
            .is_empty()
    );
}

#[tokio::test]
async fn repacking_preserves_whole_source_chunks_at_size_boundaries() {
    let store = SqliteStore::open(":memory:").await.expect("database");
    let mut meta = meta();
    store.create(meta.clone(), None).await.expect("session");
    let turn = TurnId::new();
    let events = batch(
        &mut meta,
        [
            EventKind::TurnStarted { turn },
            chunk("a".repeat(MAX_BLOCK_BYTES * 3 / 8)),
            chunk("b".repeat(MAX_BLOCK_BYTES * 3 / 8)),
            chunk("c".repeat(MAX_BLOCK_BYTES * 3 / 8)),
            finish(turn),
        ],
    );
    store
        .commit(meta.clone(), &events, None)
        .await
        .expect("turn");
    // Legal chunks left by an older packer: merging a prefix of the second
    // chunk would cross the limit and leave overlapping source coverage.
    for group in [&events[..2], &events[2..]] {
        let encoded = codec::encode(group).expect("bounded source chunk");
        let first = raw(group.first().expect("first").cursor).expect("cursor");
        let last = raw(group.last().expect("last").cursor).expect("cursor");
        sql::statement("DELETE FROM history_chunks WHERE first_cursor BETWEEN ?1 AND ?2")
            .bind(first)
            .bind(last)
            .exec(&mut store.db.clone())
            .await
            .expect("replace source");
        sql::statement("INSERT INTO history_chunks VALUES (?1, ?2, ?3, 1, 1, ?4, ?5, ?6)")
            .bind(meta.id.to_string())
            .bind(first)
            .bind(last)
            .bind(encoded.codec)
            .bind(encoded.raw_bytes as i64)
            .bind(encoded.payload)
            .exec(&mut store.db.clone())
            .await
            .expect("source chunk");
    }
    sql::statement("DELETE FROM history_state")
        .exec(&mut store.db.clone())
        .await
        .expect("clear derived checkpoint");
    assert_eq!(
        store
            .window(meta.id, None, 1)
            .await
            .expect("reindex whole chunks")
            .events,
        events
    );
    store
        .pack_turn(meta.id, turn, meta.revision)
        .await
        .expect("repack");
    assert_eq!(count(&store, "history_chunks").await, 2);
    assert_eq!(
        store
            .page(meta.id, None, 10)
            .await
            .expect("unchanged stream")
            .events,
        events
    );
}
