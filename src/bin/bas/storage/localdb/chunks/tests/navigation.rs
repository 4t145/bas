use super::*;

#[tokio::test]
async fn packed_navigation_rebuilds_metadata_and_never_reads_unrequested_bodies() {
    let store = SqliteStore::open(":memory:").await.expect("open");
    let memory = Memory::new(NonZeroUsize::new(64).expect("capacity"));
    let mut meta = meta();
    store.create(meta.clone(), None).await.expect("create");
    memory.create(meta.clone(), None).await.expect("reference");
    let mut all = Vec::new();
    for index in 0..3 {
        let turn = TurnId::new();
        let events = batch(
            &mut meta,
            [
                EventKind::TurnStarted { turn },
                EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
                    acp::ContentChunk::new(format!("prompt-{index}").into()),
                )),
                chunk("packed answer ".repeat(1000)),
                finish(turn),
            ],
        );
        store
            .commit(meta.clone(), &events, None)
            .await
            .expect("commit");
        memory
            .commit(meta.clone(), &events, None)
            .await
            .expect("reference commit");
        store
            .pack_turn(meta.id, turn, meta.revision)
            .await
            .expect("pack");
        all.extend(events);
    }
    assert_eq!(count(&store, "history_chunks").await, 3);
    let expected = memory.overview(meta.id).await.expect("reference overview");
    assert_eq!(
        store.overview(meta.id).await.expect("packed overview"),
        expected
    );
    for cursor in [1, 5, 9] {
        assert_eq!(
            store
                .window_at(meta.id, Cursor::from_raw(cursor))
                .await
                .expect("packed location"),
            memory
                .window_at(meta.id, Cursor::from_raw(cursor))
                .await
                .expect("reference location")
        );
    }
    for table in ["history_state", "history_segments"] {
        sql::statement(format!("DELETE FROM {table}"))
            .exec(&mut store.db.clone())
            .await
            .expect("clear derived catalog");
    }
    assert_eq!(
        store.overview(meta.id).await.expect("rebuilt overview"),
        expected
    );
    assert_eq!(
        store
            .page(meta.id, None, 100)
            .await
            .expect("unaltered events and timestamps")
            .events,
        all
    );
    assert_eq!(count(&store, "history_chunks").await, 3);
    assert_eq!(
        store
            .window_at(meta.id, Cursor::from_raw(5))
            .await
            .expect("rebuilt middle"),
        memory
            .window_at(meta.id, Cursor::from_raw(5))
            .await
            .expect("reference middle")
    );

    // If either query decodes unrelated bodies, corruption makes it fail. The
    // complete catalog remains readable after its one-time rebuild.
    sql::statement("UPDATE history_chunks SET payload = X'00' WHERE first_cursor = 5")
        .exec(&mut store.db.clone())
        .await
        .expect("corrupt unrequested middle");
    assert_eq!(
        store
            .overview(meta.id)
            .await
            .expect("metadata-only overview"),
        expected
    );
    assert_eq!(
        store
            .window_at(meta.id, Cursor::from_raw(1))
            .await
            .expect("direct oldest skips middle"),
        memory
            .window_at(meta.id, Cursor::from_raw(1))
            .await
            .expect("oldest")
    );
    assert_eq!(
        store
            .window_at(meta.id, Cursor::from_raw(9))
            .await
            .expect("direct newest skips middle"),
        memory
            .window_at(meta.id, Cursor::from_raw(9))
            .await
            .expect("newest")
    );
    assert!(store.window_at(meta.id, Cursor::from_raw(5)).await.is_err());
}
