use std::path::{Path, PathBuf};

use super::Store;

fn page<T>(page: &super::Page<'_, T>) -> Vec<String> {
    page.sessions
        .iter()
        .map(|session| session.id.clone())
        .collect()
}

#[test]
fn session_pages_are_stable_and_cursor_checked() {
    let mut store = Store::<()>::new(2);
    let cwd = PathBuf::from("/workspace");
    let first = store.create(cwd.clone(), Vec::new(), 0);
    let second = store.create(cwd.clone(), Vec::new(), 0);
    let third = store.create(cwd.clone(), Vec::new(), 0);
    let elsewhere = store.create(PathBuf::from("/elsewhere"), Vec::new(), 1);

    let page_one = store.page(Some(&cwd), None).unwrap();
    assert_eq!(page(&page_one), [first.clone(), second.clone()]);
    assert_eq!(page_one.next_cursor.as_deref(), Some("2"));

    // The cwd filter is applied before pagination, so offset 2 addresses the
    // third matching session rather than the third global session.
    let page_two = store
        .page(
            Some(Path::new("/workspace")),
            page_one.next_cursor.as_deref(),
        )
        .unwrap();
    assert_eq!(page(&page_two), vec![third.clone()]);
    assert_eq!(page_two.next_cursor, None);

    let all = store.page(None, Some("2")).unwrap();
    assert_eq!(page(&all), [third.clone(), elsewhere]);
    assert!(store.page(None, Some("nope")).is_err());
}

#[test]
fn fork_copies_history_and_delete_removes_the_session() {
    let mut store = Store::<u8>::new(10);
    let cwd = PathBuf::from("/workspace");
    let source = store.create(cwd.clone(), vec![PathBuf::from("/extra")], 2);
    store.record(&source, 7);
    store.resume(&source, &cwd, Vec::new(), 0).unwrap();

    let fork = store.fork(&source, cwd.clone(), Vec::new(), 0).unwrap();
    assert_ne!(source, fork);
    assert_eq!(store.get(&fork).unwrap().history, [7]);

    store.delete(&fork).unwrap();
    assert!(store.get(&fork).is_err());
    assert!(store.get(&source).is_ok());
}

#[test]
fn busy_sessions_reject_overlapping_prompts_and_close_cancels_work() {
    let mut store = Store::<()>::new(10);
    let id = store.create(PathBuf::from("/workspace"), Vec::new(), 0);
    store.begin_prompt(&id).unwrap();
    assert!(store.begin_prompt(&id).is_err());
    store.close(&id).unwrap();
    assert!(store.is_cancelled(&id));
    store.finish_prompt(&id);
    assert!(!store.is_cancelled(&id));
    assert!(store.begin_prompt(&id).is_err());
}
