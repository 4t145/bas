use super::*;
use gpui_kit::{TestAppContext, test::TestWindowExt};

fn fixture(cx: &mut TestAppContext) -> (WindowHandle<Root>, WeakEntity<Viewer>, Store) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
        init(cx);
        let store = super::super::store(cx);
        let handle = open(cx).expect("log window");
        (handle, cx.global::<OpenWindow>().viewer.clone(), store)
    })
}

fn deliver(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(UPDATE_INTERVAL);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn store_updates_live_view_and_clear_keeps_collecting(cx: &mut TestAppContext) {
    let (handle, viewer, store) = fixture(cx);
    store.record(Level::Info, "frontend", "first");
    deliver(cx);
    cx.update(|cx| {
        viewer
            .update(cx, |view, _| {
                assert_eq!(view.snapshot.entries.len(), 1);
                assert_eq!(view.snapshot.entries[0].message, "first");
            })
            .expect("viewer")
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("logs-pause").is_none());
        assert_eq!(
            window.find("logs-clear").label(),
            Some(crate::i18n::tr!("logs-clear"))
        );
        window.click("logs-clear", cx);
    })
    .expect("clear");
    assert!(store.snapshot().entries.is_empty());
    cx.update(|cx| {
        viewer
            .update(cx, |view, _| assert!(view.visible.is_empty()))
            .expect("cleared viewer")
    });
    store.record(Level::Warn, "frontend", "after clearing");
    deliver(cx);
    cx.update(|cx| {
        viewer
            .update(cx, |view, _| {
                assert_eq!(view.snapshot.entries.len(), 1);
                assert_eq!(view.snapshot.entries[0].message, "after clearing");
            })
            .expect("collection continues")
    });
}

#[test]
fn clear_list_icon_is_bundled() {
    use gpui_kit::AssetSource;
    assert!(
        crate::assets::Assets
            .load("icons/list-x.svg")
            .expect("icon")
            .is_some()
    );
}

#[gpui_kit::test]
fn toolbar_actions_keep_the_same_geometry_with_large_ui_text(cx: &mut TestAppContext) {
    let (handle, _, _) = fixture(cx);
    for font_size in [15., 24.] {
        cx.update(|cx| gpui_kit::component::Theme::global_mut(cx).font_size = px(font_size));
        cx.update_window(handle.into(), |_, window, cx| {
            window.resize(size(px(MIN_WIDTH), px(MIN_HEIGHT)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            assert!(window.try_find("logs-pause").is_none());
            let clear = window.find("logs-clear").bounds();
            let copy = window.find("logs-copy-visible").bounds();
            assert_eq!(clear.size, copy.size);
            assert_eq!(clear.top(), copy.top());
            assert_eq!(
                clear.size,
                size(
                    px(crate::ui::COMPACT_CONTROL),
                    px(crate::ui::COMPACT_CONTROL)
                )
            );
            let toolbar = window.find("logs-toolbar").bounds();
            for id in [
                "logs-level",
                "logs-target",
                "logs-clear",
                "logs-copy-visible",
            ] {
                let bounds = window.find(id).bounds();
                assert!(bounds.left() >= toolbar.left() && bounds.right() <= toolbar.right());
            }
            assert!(window.find("logs-target").bounds().right() < clear.left());
            assert_eq!(
                window.find("logs-clear").label(),
                Some(crate::i18n::tr!("logs-clear"))
            );
        })
        .expect("scaled toolbar");
    }
}

#[gpui_kit::test]
fn scrolling_up_stops_following_but_not_log_collection(cx: &mut TestAppContext) {
    let (handle, viewer, store) = fixture(cx);
    for index in 0..100 {
        store.record(Level::Info, "frontend", format!("entry {index}"));
    }
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        viewer
            .update(cx, |view, cx| {
                assert_eq!(view.scroll.is_scrolled_to_end(), Some(true));
                view.scroll.scroll_to_item(0, gpui_kit::ScrollStrategy::Top);
                cx.notify();
            })
            .expect("scroll to earlier logs");
        window.render_frame(cx);
    })
    .expect("view logs");
    store.record(Level::Info, "frontend", "arrived while reading");
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        viewer
            .update(cx, |view, cx| {
                assert_eq!(view.snapshot.entries.len(), 101);
                assert_eq!(view.scroll.is_scrolled_to_end(), Some(false));
                assert_eq!(view.scroll.logical_scroll_top_index(), 0);
                view.scroll.scroll_to_bottom();
                cx.notify();
            })
            .expect("return to newest logs");
        window.render_frame(cx);
    })
    .expect("retain scroll position");
    store.record(Level::Info, "frontend", "follow again");
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        viewer
            .update(cx, |view, _| {
                assert_eq!(view.snapshot.entries.len(), 102);
                assert_eq!(view.scroll.is_scrolled_to_end(), Some(true));
            })
            .expect("following resumes");
    })
    .expect("newest log remains visible");
}

#[gpui_kit::test]
fn search_level_and_clipboard_preserve_retained_multiline_text(cx: &mut TestAppContext) {
    let (handle, viewer, store) = fixture(cx);
    let message = format!(
        "first\r\nsecond\tline\n{}끝",
        "界".repeat(PREVIEW_CHARS * 3)
    );
    let selected = store.record(Level::Warn, "Frontend.Network", message.clone());
    store.record(Level::Info, "Frontend.Network", "other");
    store.record(Level::Warn, "other", "unrelated");
    deliver(cx);
    let expected = store.snapshot().entries[0].text();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("NETWORK", cx);
        window.click("logs-level", cx);
        for _ in 0..5 {
            window.press("down", cx);
        }
        window.press("enter", cx);
    })
    .expect("search window");
    cx.run_until_parked();
    cx.update(|cx| {
        viewer
            .update(cx, |view, cx| {
                assert_eq!(view.visible.len(), 1);
                assert_eq!(view.visible_text(), expected);
                view.selected = Some(selected);
                cx.notify();
                assert!(preview(&message).len() < message.len());
                view.copy_selected(cx);
                assert_eq!(
                    cx.read_from_clipboard().and_then(|item| item.text()),
                    Some(expected.clone())
                );
                view.copy_visible(cx);
                assert_eq!(
                    cx.read_from_clipboard().and_then(|item| item.text()),
                    Some(expected.clone())
                );
            })
            .expect("copy")
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("logs-detail").visible());
        window.click("logs-copy-selected", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(expected.clone())
        );
    })
    .expect("copy button");
    store.record(Level::Warn, "Frontend.Network", "next\nentry");
    deliver(cx);
    let entries = store.snapshot().entries;
    let visible = format!("{}\n{}", entries[0].text(), entries[3].text());
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("logs-copy-visible", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(visible)
        );
    })
    .expect("copy multiple filtered entries");
}

#[gpui_kit::test]
fn target_dropdown_and_details_fit_minimum_width(cx: &mut TestAppContext) {
    let (handle, viewer, store) = fixture(cx);
    let target = "a::very_long_target".repeat(20);
    let id = store.record(Level::Warn, target.clone(), "long message\n".repeat(300));
    store.record(Level::Warn, "z::other", "other");
    deliver(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.resize(size(px(MIN_WIDTH), px(MIN_HEIGHT)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        assert!(window.try_find("logs-detail").is_none());
        window.click("logs-target", cx);
        window.press("down", cx); // All targets
        window.press("down", cx); // First (sorted) tracing target
        window.press("enter", cx);
    })
    .expect("target dropdown");
    cx.update(|cx| {
        viewer
            .update(cx, |view, _| {
                assert_eq!(view.target.as_deref(), Some(target.as_str()));
                assert_eq!(view.visible.len(), 1);
            })
            .expect("filtered target");
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let toolbar = window.find("logs-toolbar").bounds();
        for control in [
            "logs-level",
            "logs-target",
            "logs-clear",
            "logs-copy-visible",
        ] {
            let bounds = window.find(control).bounds();
            assert!(bounds.left() >= toolbar.left());
            assert!(bounds.right() <= toolbar.right());
            assert!(bounds.top() >= toolbar.top());
            assert!(bounds.bottom() <= toolbar.bottom());
        }
        assert!(toolbar.size.height <= px(52.));
        window.click(("log-entry", id), cx);
        window.render_frame(cx);
        let detail = window.find("logs-detail").bounds();
        assert!(detail.size.height <= px(DETAIL_HEIGHT));
        assert!(
            detail.right() <= px(MIN_WIDTH),
            "{detail:?}; toolbar {toolbar:?}"
        );
        window.click("logs-copy-selected", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(store.snapshot().entries[0].text())
        );
        window.click("logs-close-detail", cx);
        window.render_frame(cx);
        assert!(window.try_find("logs-detail").is_none());
    })
    .expect("minimum width layout and full detail copy");
}

#[gpui_kit::test]
fn shift_l_opens_logs_but_shift_d_does_not(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
        init(cx);
    });
    let host = cx.open_window(size(px(800.), px(600.)), |_, _| gpui_kit::Empty);
    cx.update_window(host.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-shift-d"
            } else {
                "ctrl-shift-d"
            },
            cx,
        );
    })
    .expect("old shortcut");
    cx.run_until_parked();
    cx.update(|cx| assert!(!cx.has_global::<OpenWindow>()));
    cx.update_window(host.into(), |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-shift-l"
            } else {
                "ctrl-shift-l"
            },
            cx,
        );
    })
    .expect("logs shortcut");
    cx.run_until_parked();
    cx.update(|cx| assert!(cx.global::<OpenWindow>().viewer.upgrade().is_some()));
}

#[gpui_kit::test]
fn singleton_close_reopen_retains_store_and_releases_viewer(cx: &mut TestAppContext) {
    let (first, viewer, store) = fixture(cx);
    let again = cx.update(|cx| open(cx).expect("existing window"));
    assert_eq!(first.window_id(), again.window_id());
    store.record(Level::Error, "frontend", "retained across close");
    cx.update_window(first.into(), |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-shift-l"
            } else {
                "ctrl-shift-l"
            },
            cx,
        );
    })
    .expect("reveal shortcut");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.global::<OpenWindow>().handle.window_id(),
            first.window_id()
        )
    });
    cx.update_window(first.into(), |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
    })
    .expect("close");
    cx.run_until_parked();
    cx.update(|_| {
        assert!(
            viewer.upgrade().is_none(),
            "closing drops the view and watch task"
        )
    });
    store.record(Level::Debug, "frontend", "recorded without a window");
    let second = cx.update(|cx| open(cx).expect("reopened window"));
    assert_ne!(first.window_id(), second.window_id());
    cx.update(|cx| {
        let viewer = cx.global::<OpenWindow>().viewer.clone();
        viewer
            .update(cx, |view, _| assert_eq!(view.snapshot.entries.len(), 2))
            .expect("new viewer");
    });
}
