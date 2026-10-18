use super::*;
use workspace_navigation::{View, restore::Store};

#[gpui_kit::test]
fn bookmarks_shortcut_and_sidebar_views_preserve_the_current_conversation(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let current = session();
    let other = session();
    let mut archived = session();
    archived.archived = true;
    attach(&fixture, current.clone(), cx);
    let state = tempfile::tempdir().expect("navigation");
    let path = state.path().join("window.json");
    let mut store = Store::open(path.clone());
    store.set_pinned(other.id, true).expect("existing pin");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries = vec![current.clone(), other.clone(), archived.clone()];
            view.attach_restore(Some(store));
            view.navigation.restore.on_connect = false;
            view.composer.input.update(cx, |input, cx| {
                input.set_value("keep draft", window, cx);
                input.focus(window, cx);
            });
        });
        window.render_frame(cx);
        let buttons =
            ["bookmarked-list", "history-list", "archived-list"].map(|id| window.find(id).bounds());
        for pair in buttons.windows(2) {
            assert!(pair[0].right() <= pair[1].left());
            assert_eq!(pair[0].size, pair[1].size);
        }
        window.press("ctrl-d", cx);
        assert!(
            fixture
                .view
                .read(cx)
                .navigation
                .bookmarks
                .contains(&current.id)
        );
        assert!(fixture.view.read(cx).navigation.pinned.contains(&other.id));
        window.render_frame(cx);
        assert!(
            window
                .find(format!("thread-bookmark-{}", current.id))
                .visible()
        );
    })
    .expect("bookmark shortcut from focused composer");
    assert_eq!(
        Store::open(path.clone()).bookmarks(),
        HashSet::from([current.id])
    );
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.click("history-list", cx);
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).navigation.view, View::History);
        assert!(window.find("history-today").visible());
        assert!(
            window
                .find(format!("thread-workspace-{}", current.id))
                .visible()
        );
        assert!(
            window
                .try_find(format!("session-{}", archived.id))
                .is_none()
        );
        window.click("bookmarked-list", cx);
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).navigation.view, View::Bookmarks);
        assert!(window.find(format!("session-{}", current.id)).visible());
        assert!(window.try_find(format!("session-{}", other.id)).is_none());
        assert_eq!(
            crate::i18n::visible_text(
                window
                    .find("thread-bookmarks-title")
                    .label()
                    .expect("count")
            ),
            "书签 · 1"
        );
        window.hover(format!("session-{}", current.id), cx);
        assert!(
            window
                .try_find(format!("bookmark-{}", current.id))
                .is_none(),
            "bookmark changes are offered by the context menu and shortcut, not row actions"
        );
        window.press("ctrl-d", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).navigation.bookmarks.is_empty());
        assert_eq!(fixture.view.read(cx).selected.id, Some(current.id));
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "keep draft"
        );
        window.click("archived-list", cx);
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).navigation.view, View::Archived);
        assert!(window.find(format!("session-{}", archived.id)).visible());
        window.click("archived-list", cx);
        assert_eq!(fixture.view.read(cx).navigation.view, View::Workspaces);
        assert_eq!(fixture.view.read(cx).selected.id, Some(current.id));
    })
    .expect("sidebar view switching");
    assert!(Store::open(path).bookmarks().is_empty());
    assert!(
        !std::iter::from_fn(|| fixture.commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Create(_) | Command::Archive { .. } | Command::Delete(_) | Command::Cancel(_)
        )),
        "navigation and bookmarking do not create, archive, delete or stop sessions"
    );
}

#[gpui_kit::test]
fn bookmark_collection_keeps_archived_threads_and_filters_search(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut saved = session();
    saved.archived = true;
    saved.title = Some("Saved conversation".into());
    let other = session();
    attach(&fixture, other.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries = vec![saved.clone(), other];
            view.toggle_bookmark(saved.id, cx);
        });
        window.render_frame(cx);
        window.click("bookmarked-list", cx);
        window.render_frame(cx);
        assert!(window.find(format!("session-{}", saved.id)).visible());
        fixture.view.update(cx, |view, cx| {
            view.navigation
                .search
                .update(cx, |input, cx| input.set_value("no match", window, cx));
        });
        window.render_frame(cx);
        assert!(window.try_find(format!("session-{}", saved.id)).is_none());
        assert!(
            fixture
                .view
                .read(cx)
                .navigation
                .bookmarks
                .contains(&saved.id)
        );
    })
    .expect("bookmarks are independent of archive and search");
}
