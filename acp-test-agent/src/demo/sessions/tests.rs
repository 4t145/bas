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
fn fork_inherits_content_but_owns_independent_session_state() {
    let mut store = Store::<String>::new(2);
    let source = store.create(PathBuf::from("/source"), Vec::new(), 0);
    {
        let session = store.get_mut(&source).expect("source exists");
        session.title = "Original".into();
        session.mode = crate::demo::MODE_CODE.into();
        session
            .config
            .apply_setting(Some("deep"))
            .expect("valid model");
    }
    store.record(&source, "first".into());
    store.close(&source).expect("source can close");
    let cwd = PathBuf::from("/fork");
    let extra = vec![PathBuf::from("/extra")];
    let fork = store
        .fork(&source, cwd.clone(), extra.clone(), 2)
        .expect("closed source can fork");
    let session = store.get(&fork).expect("fork exists");
    assert_ne!(session.id, source);
    assert_eq!(session.title, "Original (fork)");
    assert_eq!(session.cwd, cwd);
    assert_eq!(session.additional_directories, extra);
    assert_eq!(session.mcp_servers, 2);
    assert_eq!(session.mode, crate::demo::MODE_CODE);
    assert_eq!(session.config.model, crate::demo::MODEL_DEEP);
    assert_eq!(session.history, ["first"]);
    assert!(session.active);
    assert!(!session.busy);
    assert!(!session.cancelled);

    store.record(&fork, "second".into());
    let original = store.get(&source).expect("source remains");
    assert_eq!(original.history, ["first"]);
    assert!(!original.active);
}

#[test]
fn fork_rejects_unknown_and_busy_sources_without_allocating_a_session() {
    use super::Error;
    let mut store = Store::<()>::new(2);
    let cwd = PathBuf::from("/workspace");
    assert_eq!(
        store.fork("missing", cwd.clone(), Vec::new(), 0),
        Err(Error::UnknownSession("missing".into()))
    );
    let source = store.create(cwd.clone(), Vec::new(), 0);
    store.begin_prompt(&source).expect("start source prompt");
    assert_eq!(
        store.fork(&source, cwd, Vec::new(), 0),
        Err(Error::BusySession(source))
    );
    assert_eq!(
        store.page(None, None).expect("session list").sessions.len(),
        1
    );
}
