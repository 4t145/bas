use super::*;
use std::collections::{BTreeMap, BTreeSet};

mod marker_bookmarks;

#[test]
fn saved_reads_advance_monotonically_and_ignore_zero_cursors() {
    let first = SessionId::new();
    let second = SessionId::new();
    let unread = SessionId::new();
    let mut saved = Saved::default();
    assert!(saved.merge_reads(&HashMap::from([(first, 9), (second, 2)])));
    assert!(saved.merge_reads(&HashMap::from([(first, 4), (second, 7), (unread, 0)])));
    assert_eq!(
        saved.seen_replies,
        BTreeMap::from([(first, 9), (second, 7)])
    );
    assert!(!saved.merge_reads(&HashMap::from([(first, 9), (second, 1), (unread, 0)])));
}

#[test]
fn saved_read_removal_clears_all_target_markers_but_keeps_other_sections() {
    let first = SessionId::new();
    let marker_only = SessionId::new();
    let retained = SessionId::new();
    let mut saved = Saved {
        thread: Some(first),
        pinned: [first].into(),
        bookmarks: [first].into(),
        ..Default::default()
    };
    saved.merge_reads(&HashMap::from([(first, 9), (retained, 7)]));
    for session in [first, marker_only, retained] {
        saved.set_marker_bookmarked(session, 3, true);
    }
    assert!(saved.forget_reads(&[first, marker_only]));
    assert!(!saved.forget_reads(&[first, marker_only]));
    assert_eq!(saved.seen_replies, BTreeMap::from([(retained, 7)]));
    assert_eq!(
        saved.marker_bookmarks,
        BTreeMap::from([(retained, BTreeSet::from([3]))])
    );
    assert_eq!(saved.thread, Some(first));
    assert_eq!(saved.pinned, BTreeSet::from([first]));
    assert_eq!(saved.bookmarks, BTreeSet::from([first]));
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn queued_navigation_is_ordered_detached_and_merges_other_sections(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let other_path = directory.path().join("other-server.json");
    let first = SessionId::new();
    let last = SessionId::new();
    let executor = cx.background_executor.clone();
    let (release, gate) = futures::channel::oneshot::channel::<()>();
    let mut pending = Some(
        executor
            .spawn(async move {
                gate.await.expect("release write");
            })
            .shared(),
    );
    for (thread, seen) in [
        (first, HashMap::from([(first, 4)])),
        (last, HashMap::from([(first, 9), (last, 2)])),
    ] {
        let path = path.clone();
        enqueue(&mut pending, &executor, move || {
            Store::open(path)
                .save_navigation(Some(Some(thread)), &seen)
                .expect("navigation");
        });
    }
    let other = other_path.clone();
    enqueue(&mut pending, &executor, move || {
        Store::open(other)
            .save_navigation(Some(Some(first)), &HashMap::new())
            .expect("other server");
    });
    cx.run_until_parked();
    assert!(
        !path.exists(),
        "a blocked predecessor prevents out-of-order writes"
    );
    let mut concurrent = Store::open(path.clone());
    concurrent.set_bookmarked(first, true).expect("bookmark");
    concurrent.set_pinned(last, true).expect("pin");
    concurrent.set_workspace_hidden(None, true).expect("hide");
    concurrent
        .save_reads(&HashMap::from([(last, 7)]))
        .expect("other window read");
    // Dropping the window's completion handle must not cancel queued transactions.
    drop(pending);
    release.send(()).expect("unblock write");
    cx.run_until_parked();
    let restored = Store::open(path);
    assert_eq!(restored.thread(), Some(last));
    assert_eq!(
        restored.seen_replies(),
        HashMap::from([(first, 9), (last, 7)])
    );
    assert_eq!(restored.bookmarks(), HashSet::from([first]));
    assert_eq!(restored.pinned(), HashSet::from([last]));
    assert_eq!(restored.hidden_workspaces(), HashSet::from([None]));
    let other = Store::open(other_path);
    assert_eq!(other.thread(), Some(first));
    assert!(other.seen_replies().is_empty());
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn queued_removal_follows_older_read_snapshots(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().expect("state directory");
    let path = directory.path().join("window.json");
    let id = SessionId::new();
    let executor = cx.background_executor.clone();
    let mut pending = None;
    let before = path.clone();
    enqueue(&mut pending, &executor, move || {
        Store::open(before)
            .save_navigation(Some(Some(id)), &HashMap::from([(id, 9)]))
            .expect("read");
    });
    let after = path.clone();
    enqueue(&mut pending, &executor, move || {
        Store::open(after).forget_reads(&[id]).expect("forget");
    });
    cx.run_until_parked();
    assert!(Store::open(path).seen_replies().is_empty());
}

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
