use super::*;

#[test]
fn bookmarks_persist_independently_and_stale_writers_preserve_them() {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let mut first = Store::open(path.clone());
    let mut second = Store::open(path.clone());
    let bookmarked = SessionId::new();
    let pinned = SessionId::new();
    assert!(first.bookmarks().is_empty());
    first.set_bookmarked(bookmarked, true).expect("bookmark");
    second
        .set_pinned(pinned, true)
        .expect("pin from stale reader");
    second.save(Some(pinned)).expect("selection");
    let restored = Store::open(path.clone());
    assert_eq!(restored.bookmarks(), HashSet::from([bookmarked]));
    assert_eq!(restored.pinned(), HashSet::from([pinned]));
    first
        .set_bookmarked(bookmarked, false)
        .expect("remove bookmark");
    let restored = Store::open(path);
    assert!(restored.bookmarks().is_empty());
    assert_eq!(restored.thread(), Some(pinned));
    assert_eq!(restored.pinned(), HashSet::from([pinned]));
}

#[test]
fn completed_write_releases_lock_while_a_duplicate_descriptor_remains_open() {
    let directory = tempfile::tempdir().expect("state directory");
    let mut store = Store::open(directory.path().join("window.json"));
    let lock = store.lock().expect("first transaction");
    let inherited = lock.0.try_clone().expect("duplicate descriptor");
    assert!(matches!(store.save(None), Err(Error::Busy)));
    drop(lock);
    store
        .save(None)
        .expect("completed transaction releases its lock");
    drop(inherited);
}

#[test]
fn hidden_workspaces_persist_without_overwriting_other_window_state() {
    let dir = tempfile::tempdir().expect("state directory");
    let path = dir.path().join("window.json");
    let mut a = Store::open(path.clone());
    let mut b = Store::open(path.clone());
    let thread = SessionId::new();
    a.set_workspace_hidden(Some("/workspace".into()), true)
        .expect("hide");
    b.set_pinned(thread, true).expect("pin from stale reader");
    b.save(Some(thread)).expect("selection");
    let restored = Store::open(path.clone());
    assert!(
        restored
            .hidden_workspaces()
            .contains(&Some("/workspace".into()))
    );
    assert!(restored.pinned().contains(&thread));
    a.set_workspace_hidden(Some("/workspace".into()), false)
        .expect("show");
    assert!(Store::open(path).hidden_workspaces().is_empty());
}

#[test]
fn concurrent_writers_are_rejected_before_read_modify_write() {
    let dir = tempfile::tempdir().expect("state directory");
    let path = dir.path().join("state.json");
    let mut first = Store::open(path.clone());
    let mut second = Store::open(path.clone());
    let lock = first.lock().expect("hold transaction lock");
    let id = SessionId::new();
    assert!(matches!(second.set_pinned(id, true), Err(Error::Busy)));
    assert!(matches!(second.set_bookmarked(id, true), Err(Error::Busy)));
    assert!(matches!(second.save(Some(id)), Err(Error::Busy)));
    assert!(matches!(
        second.save_reads(&HashMap::from([(id, 4)])),
        Err(Error::Busy)
    ));
    assert!(!path.exists(), "a contending writer must not replace state");
    drop(lock);
    first
        .set_pinned(id, true)
        .expect("complete pin transaction");
    second
        .save(Some(id))
        .expect("retry selection against latest state");
    assert!(Store::open(path).pinned().contains(&id));
}

#[test]
fn pins_persist_and_legacy_selection_files_remain_readable() {
    let dir = tempfile::tempdir().expect("state directory");
    let path = dir.path().join("state.json");
    let thread = SessionId::new();
    std::fs::write(&path, format!(r#"{{"thread":"{thread}"}}"#)).expect("legacy state");
    let mut store = Store::open(path.clone());
    assert_eq!(store.thread(), Some(thread));
    assert!(store.pinned().is_empty());
    assert!(store.seen_replies().is_empty());
    store.set_pinned(thread, true).expect("pin");
    store.save(None).expect("selection change");
    let mut restored = Store::open(path);
    assert!(restored.pinned().contains(&thread));
    assert_eq!(restored.thread(), None);
    restored.set_pinned(thread, false).expect("unpin");
    assert!(restored.pinned().is_empty());
}

#[test]
fn stale_selection_writer_preserves_newer_pin_changes() {
    let dir = tempfile::tempdir().expect("state directory");
    let path = dir.path().join("state.json");
    let first = SessionId::new();
    let second = SessionId::new();
    let mut selection_writer = Store::open(path.clone());
    let mut pin_writer = Store::open(path.clone());
    pin_writer.set_pinned(first, true).expect("pin first");
    selection_writer
        .save(Some(second))
        .expect("save newer selection");
    let current = Store::open(path);
    assert_eq!(current.thread(), Some(second));
    assert!(current.pinned().contains(&first));
}

#[test]
fn malformed_state_is_not_overwritten_on_pin_or_selection_change() {
    let dir = tempfile::tempdir().expect("state directory");
    let path = dir.path().join("state.json");
    std::fs::write(&path, "{invalid").expect("invalid state");
    let mut store = Store::open(path.clone());
    assert!(store.set_pinned(SessionId::new(), true).is_err());
    assert!(store.save(None).is_err());
    assert!(
        store
            .save_reads(&HashMap::from([(SessionId::new(), 7)]))
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(path).expect("state preserved"),
        "{invalid"
    );
    assert!(store.pinned().is_empty());
}

#[test]
fn read_frontiers_merge_monotonically_without_overwriting_other_state() {
    let dir = tempfile::tempdir().expect("state directory");
    let path = dir.path().join("state.json");
    let first = SessionId::new();
    let second = SessionId::new();
    let mut a = Store::open(path.clone());
    let mut b = Store::open(path.clone());
    a.save_reads(&HashMap::from([(first, 9)]))
        .expect("read first");
    b.save_reads(&HashMap::from([(first, 5), (second, 7)]))
        .expect("merge stale writer");
    a.set_pinned(second, true).expect("pin preserves reads");
    b.save(Some(second)).expect("selection preserves reads");
    let mut restored = Store::open(path.clone());
    assert_eq!(
        restored.seen_replies(),
        HashMap::from([(first, 9), (second, 7)])
    );
    assert_eq!(restored.thread(), Some(second));
    assert!(restored.pinned().contains(&second));
    restored
        .forget_reads(&[first])
        .expect("remove only deleted session");
    assert_eq!(
        Store::open(path).seen_replies(),
        HashMap::from([(second, 7)])
    );
    assert!(
        Store::open(dir.path().join("other-server.json"))
            .seen_replies()
            .is_empty()
    );
}
