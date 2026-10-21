use super::*;
use gpui_kit::ScrollStrategy;

fn selection(viewer: &WeakEntity<Viewer>, cx: &App) -> Option<u64> {
    viewer.upgrade().expect("viewer").read(cx).selected
}

fn start_at_top(viewer: &WeakEntity<Viewer>, window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    viewer
        .update(cx, |view, cx| {
            view.scroll.scroll_to_item(0, ScrollStrategy::Top);
            cx.notify();
        })
        .expect("scroll to first entry");
    window.render_frame(cx);
}

#[gpui_kit::test]
fn arrows_work_from_search_and_clicked_rows_without_stealing_menu_keys(cx: &mut TestAppContext) {
    let (handle, viewer, store) = fixture(cx);
    let ids: Vec<_> = (0..80)
        .map(|index| store.record(Level::Info, "frontend", format!("entry {index}")))
        .collect();
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        start_at_top(&viewer, window, cx);
        window.input("entry", cx);
    })
    .expect("search");
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), Some(ids[0]));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), Some(ids[1]));
        window.press("up", cx);
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), Some(ids[0]));
        assert_eq!(
            viewer
                .upgrade()
                .expect("viewer")
                .read(cx)
                .search
                .read(cx)
                .value(),
            "entry"
        );
        window.click(("log-entry", ids[4]), cx);
        window.render_frame(cx);
        viewer
            .update(cx, |view, _| assert!(view.list_focus.is_focused(window)))
            .expect("list focus");
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), Some(ids[3]));
        window.click("logs-copy-selected", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(store.snapshot().entries[3].text())
        );
        assert!(window.try_find("logs-copy-visible").is_none());
        window.click("logs-level", cx);
        window.press("down", cx);
        window.press("down", cx);
        assert_eq!(
            selection(&viewer, cx),
            Some(ids[3]),
            "filter menu owns its arrow keys"
        );
        window.press("escape", cx);
    })
    .expect("keyboard selection and detail copy");
}

#[gpui_kit::test]
fn page_keys_use_the_rendered_viewport_and_keep_selection_visible(cx: &mut TestAppContext) {
    let (handle, viewer, store) = fixture(cx);
    let ids: Vec<_> = (0..120)
        .map(|index| store.record(Level::Info, "frontend", format!("entry {index}")))
        .collect();
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        start_at_top(&viewer, window, cx);
        window.click(("log-entry", ids[0]), cx);
        for height in [MIN_HEIGHT, HEIGHT] {
            window.resize(size(px(MIN_WIDTH), px(height)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            let viewport = window.find("logs-list-viewport").bounds();
            let rows = (f32::from(viewport.size.height) / ROW_HEIGHT).floor() as usize;
            assert!(rows > 1 && rows < ids.len());
            window.press("pagedown", cx);
            window.render_frame(cx);
            assert_eq!(selection(&viewer, cx), Some(ids[rows]));
            let row = window.find(("log-entry", ids[rows])).bounds();
            let viewport = window.find("logs-list-viewport").bounds();
            assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom());
            let offset = viewer
                .upgrade()
                .expect("viewer")
                .read(cx)
                .scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y;
            assert_eq!(offset, px(-(rows as f32 * ROW_HEIGHT)));
            window.press("pageup", cx);
            window.render_frame(cx);
            assert_eq!(selection(&viewer, cx), Some(ids[0]));
        }
        for _ in 0..20 {
            window.press("pagedown", cx);
            window.render_frame(cx);
        }
        assert_eq!(selection(&viewer, cx), ids.last().copied());
        assert!(
            window
                .find(("log-entry", *ids.last().expect("last")))
                .visible()
        );
        window.press("down", cx);
        assert_eq!(
            selection(&viewer, cx),
            ids.last().copied(),
            "no wrap at the end"
        );
        window.press("pageup", cx);
        window.render_frame(cx);
        assert_ne!(selection(&viewer, cx), ids.last().copied());
        assert!(
            window
                .find(("log-entry", selection(&viewer, cx).expect("selection")))
                .visible()
        );
    })
    .expect("page navigation");
}

#[gpui_kit::test]
fn filtering_empty_results_and_clear_never_leave_a_stale_keyboard_selection(
    cx: &mut TestAppContext,
) {
    let (handle, viewer, store) = fixture(cx);
    let first = store.record(Level::Info, "frontend", "matching first");
    store.record(Level::Info, "frontend", "other");
    let second = store.record(Level::Warn, "frontend", "matching second");
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("matching", cx);
    })
    .expect("filter");
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), Some(first));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            selection(&viewer, cx),
            Some(second),
            "navigate filtered entries only"
        );
        window.input(" nothing", cx);
    })
    .expect("no matches");
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        for key in ["up", "down", "pageup", "pagedown"] {
            window.press(key, cx);
        }
        assert_eq!(selection(&viewer, cx), None);
        assert!(window.try_find("logs-copy-selected").is_none());
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.press("backspace", cx);
    })
    .expect("empty navigation");
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert!(selection(&viewer, cx).is_some());
        window.click("logs-clear", cx);
        window.render_frame(cx);
        for key in ["up", "down", "pageup", "pagedown"] {
            window.press(key, cx);
        }
        assert_eq!(selection(&viewer, cx), None);
        assert!(window.try_find("logs-detail").is_none());
    })
    .expect("clear resets selection");
}

#[gpui_kit::test]
fn starting_from_the_bottom_selects_the_visible_latest_entry(cx: &mut TestAppContext) {
    let (handle, viewer, store) = fixture(cx);
    let ids: Vec<_> = (0..100)
        .map(|index| store.record(Level::Info, "frontend", format!("entry {index}")))
        .collect();
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), None);
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), Some(ids[99]));
        assert!(window.find(("log-entry", ids[99])).visible());
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(selection(&viewer, cx), Some(ids[98]));
        assert!(window.find(("log-entry", ids[98])).visible());
    })
    .expect("navigate from newest records");
}
