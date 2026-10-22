use super::*;

#[test]
fn old_state_defaults_to_no_marker_bookmarks() {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let session = SessionId::new();
    let old = serde_json::json!({
        "thread": session,
        "bookmarks": [session],
        "seen_replies": {(session.to_string()): 7}
    });
    fs::write(&path, serde_json::to_vec(&old).expect("old state")).expect("write old state");

    let restored = Store::open(path);
    assert!(restored.marker_bookmarks().is_empty());
    assert_eq!(restored.thread(), Some(session));
    assert_eq!(restored.bookmarks(), HashSet::from([session]));
    assert_eq!(restored.seen_replies(), HashMap::from([(session, 7)]));
}

#[test]
fn marker_bookmarks_are_distinct_from_thread_bookmarks_and_other_sessions() {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let first = SessionId::new();
    let second = SessionId::new();
    let mut store = Store::open(path.clone());
    store.set_bookmarked(first, true).expect("thread bookmark");
    store.save_marker_bookmark(first, 17, true).expect("marker");
    store
        .save_marker_bookmark(first, u64::MAX, true)
        .expect("original cursor without index conversion");
    store
        .save_marker_bookmark(second, 17, true)
        .expect("other session");
    store
        .set_bookmarked(first, false)
        .expect("unbookmark thread");
    let restored = Store::open(path.clone());
    assert!(restored.bookmarks().is_empty());
    assert_eq!(
        restored.marker_bookmarks(),
        HashMap::from([
            (first, HashSet::from([17, u64::MAX])),
            (second, HashSet::from([17])),
        ])
    );

    store.set_bookmarked(second, true).expect("thread bookmark");
    store
        .save_marker_bookmark(second, 17, false)
        .expect("remove marker");
    let restored = Store::open(path);
    assert_eq!(restored.bookmarks(), HashSet::from([second]));
    assert_eq!(
        restored.marker_bookmarks(),
        HashMap::from([(first, HashSet::from([17, u64::MAX]))])
    );
}

#[test]
fn stale_marker_writers_merge_only_the_targeted_marker() {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let first = SessionId::new();
    let second = SessionId::new();
    let mut a = Store::open(path.clone());
    let mut b = Store::open(path.clone());
    a.save_marker_bookmark(first, 17, true)
        .expect("first marker");
    b.save_marker_bookmark(first, 29, true)
        .expect("same session");
    b.save_marker_bookmark(second, 17, true)
        .expect("other session");
    a.save_marker_bookmark(first, 17, false)
        .expect("remove only one marker");
    b.set_bookmarked(first, true).expect("thread bookmark");
    b.set_pinned(second, true).expect("pin");
    b.set_workspace_hidden(None, true).expect("hide workspace");
    b.save(Some(second)).expect("selection");
    b.save_reads(&HashMap::from([(second, 93)]))
        .expect("read cursor");
    // Other window sections changed after A last read the file.
    a.save_marker_bookmark(first, 41, true)
        .expect("stale marker writer");

    let restored = Store::open(path);
    assert_eq!(
        restored.marker_bookmarks(),
        HashMap::from([
            (first, HashSet::from([29, 41])),
            (second, HashSet::from([17])),
        ])
    );
    assert_eq!(restored.thread(), Some(second));
    assert_eq!(restored.bookmarks(), HashSet::from([first]));
    assert_eq!(restored.pinned(), HashSet::from([second]));
    assert_eq!(restored.hidden_workspaces(), HashSet::from([None]));
    assert_eq!(restored.seen_replies(), HashMap::from([(second, 93)]));
}

#[test]
fn deletion_also_removes_markers_without_a_read_cursor() {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let session = SessionId::new();
    let mut stale = Store::open(path.clone());
    Store::open(path.clone())
        .save_marker_bookmark(session, 17, true)
        .expect("marker without read state");
    stale
        .forget_reads(&[session])
        .expect("forget deleted session");
    assert!(Store::open(path).marker_bookmarks().is_empty());
    assert!(stale.marker_bookmarks().is_empty());
}

#[test]
fn marker_writes_respect_the_transaction_lock_and_corrupt_state() {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let mut store = Store::open(path.clone());
    let session = SessionId::new();
    let lock = store.lock().expect("hold write lock");
    assert!(matches!(
        store.save_marker_bookmark(session, 17, true),
        Err(Error::Busy)
    ));
    assert!(!path.exists());
    drop(lock);

    fs::write(&path, "{invalid").expect("corrupt state");
    assert!(matches!(
        store.save_marker_bookmark(session, 17, true),
        Err(Error::Json(_))
    ));
    assert!(matches!(
        store.forget_reads(&[session]),
        Err(Error::Json(_))
    ));
    assert_eq!(
        fs::read_to_string(path).expect("preserved state"),
        "{invalid"
    );
    assert!(store.marker_bookmarks().is_empty());
}

#[cfg(feature = "gui-tests")]
fn blocked_queue(
    executor: &gpui_kit::BackgroundExecutor,
) -> (futures::channel::oneshot::Sender<()>, Option<PendingSave>) {
    let (release, gate) = futures::channel::oneshot::channel::<()>();
    let pending = Some(
        executor
            .spawn(async move {
                gate.await.expect("release write");
            })
            .shared(),
    );
    (release, pending)
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn rapid_marker_toggles_update_memory_immediately_and_persist_in_order(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let session = SessionId::new();
    let executor = cx.background_executor.clone();
    let (release, mut pending) = blocked_queue(&executor);
    let mut store = Store::open(path.clone());
    for (cursor, bookmarked) in [(17, true), (17, false), (41, true), (41, false), (41, true)] {
        store.queue_marker_bookmark(&mut pending, &executor, session, cursor, bookmarked);
        assert_eq!(
            store
                .marker_bookmarks()
                .get(&session)
                .is_some_and(|cursors| cursors.contains(&cursor)),
            bookmarked
        );
    }
    assert_eq!(
        store.marker_bookmarks(),
        HashMap::from([(session, HashSet::from([41]))])
    );
    cx.run_until_parked();
    assert!(!path.exists(), "no write may overtake its predecessor");
    // Merge state written by another window while the marker queue was blocked.
    Store::open(path.clone())
        .save(Some(session))
        .expect("other window selection");
    release.send(()).expect("unblock writes");
    cx.run_until_parked();
    let restored = Store::open(path);
    assert_eq!(restored.marker_bookmarks(), store.marker_bookmarks());
    assert_eq!(restored.thread(), Some(session));
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn marker_writes_survive_close_and_keep_the_captured_backend(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().expect("state directory");
    let first: Target = "tcp://127.0.0.1:8101".parse().expect("first target");
    let second: Target = "tcp://127.0.0.1:8102".parse().expect("second target");
    // Use discover's namespace rule without modifying the process's state root.
    let first_path = directory
        .path()
        .join(format!("{}.json", first.state_key().expect("key")));
    let second_path = directory
        .path()
        .join(format!("{}.json", second.state_key().expect("key")));
    assert_ne!(first_path, second_path);
    let session = SessionId::new();
    let executor = cx.background_executor.clone();
    let (release, mut pending) = blocked_queue(&executor);
    let mut store = Store::open(first_path.clone());
    store.queue_marker_bookmark(&mut pending, &executor, session, 17, true);
    store = Store::open(second_path.clone());
    assert!(store.marker_bookmarks().is_empty());
    store.queue_marker_bookmark(&mut pending, &executor, session, 29, true);
    drop(store);
    drop(pending);
    cx.run_until_parked();
    assert!(!first_path.exists());
    assert!(!second_path.exists());
    release.send(()).expect("unblock writes");
    cx.run_until_parked();
    assert_eq!(
        Store::open(first_path).marker_bookmarks(),
        HashMap::from([(session, HashSet::from([17]))])
    );
    assert_eq!(
        Store::open(second_path).marker_bookmarks(),
        HashMap::from([(session, HashSet::from([29]))])
    );
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn zero_cursor_is_neither_stored_nor_queued(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let session = SessionId::new();
    let executor = cx.background_executor.clone();
    let mut pending = None;
    let mut store = Store::open(path.clone());
    for bookmarked in [true, false] {
        store.queue_marker_bookmark(&mut pending, &executor, session, 0, bookmarked);
        store
            .save_marker_bookmark(session, 0, bookmarked)
            .expect("ignore zero");
    }
    assert!(pending.is_none());
    assert!(store.marker_bookmarks().is_empty());
    cx.run_until_parked();
    assert!(!path.exists());
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn queued_deletion_follows_marker_addition_and_preserves_other_sessions(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let deleted = SessionId::new();
    let kept = SessionId::new();
    let executor = cx.background_executor.clone();
    let (release, mut pending) = blocked_queue(&executor);
    let mut store = Store::open(path.clone());
    store.set_bookmarked(kept, true).expect("thread bookmark");
    let before = path.clone();
    enqueue(&mut pending, &executor, move || {
        Store::open(before)
            .save_navigation(
                Some(Some(kept)),
                &HashMap::from([(deleted, 17), (kept, 29)]),
            )
            .expect("queued read snapshot");
    });
    store.queue_marker_bookmark(&mut pending, &executor, deleted, 17, true);
    store.queue_marker_bookmark(&mut pending, &executor, kept, 29, true);
    store.queue_forget_reads(&mut pending, &executor, &[deleted]);
    assert_eq!(
        store.marker_bookmarks(),
        HashMap::from([(kept, HashSet::from([29]))])
    );
    drop(store);
    drop(pending);
    release.send(()).expect("unblock writes");
    cx.run_until_parked();

    let restored = Store::open(path);
    assert_eq!(
        restored.marker_bookmarks(),
        HashMap::from([(kept, HashSet::from([29]))])
    );
    assert_eq!(restored.seen_replies(), HashMap::from([(kept, 29)]));
    assert_eq!(restored.bookmarks(), HashSet::from([kept]));
    assert_eq!(restored.thread(), Some(kept));
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn queued_marker_write_does_not_overwrite_corrupt_state(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    fs::write(&path, "{invalid").expect("corrupt state");
    let session = SessionId::new();
    let executor = cx.background_executor.clone();
    let mut pending = None;
    let mut store = Store::open(path.clone());
    store.queue_marker_bookmark(&mut pending, &executor, session, 17, true);
    assert_eq!(
        store.marker_bookmarks(),
        HashMap::from([(session, HashSet::from([17]))])
    );
    cx.run_until_parked();
    assert_eq!(
        fs::read_to_string(path).expect("preserved state"),
        "{invalid"
    );
}
