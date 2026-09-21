use super::*;

#[test]
fn concurrent_writers_are_rejected_before_read_modify_write() {
    let dir = tempfile::tempdir().expect("state directory");
    let path = dir.path().join("state.json");
    let mut first = Store::open(path.clone());
    let mut second = Store::open(path.clone());
    let lock = first.lock().expect("hold transaction lock");
    let id = SessionId::new();
    assert!(matches!(second.set_pinned(id, true), Err(Error::Busy)));
    assert!(matches!(second.save(Some(id)), Err(Error::Busy)));
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
    assert_eq!(
        std::fs::read_to_string(path).expect("state preserved"),
        "{invalid"
    );
    assert!(store.pinned().is_empty());
}
