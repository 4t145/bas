use super::*;
use gpui_kit::{TestAppContext, size, test::TestWindowExt};

#[gpui_kit::test]
fn navigation_and_search_filter_settings_without_a_save_button(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings config");
    let path = directory.path().join("appearance.json");
    std::fs::write(&path, r#"{"theme_source":"builtin"}"#).expect("builtin config");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(None, Some(path), cx);
    });
    let mut view = None;
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        view = Some(settings.clone());
        Root::new(settings, window, cx)
    });
    let view = view.expect("settings view");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-save").is_none());
        window.click("settings-layout", cx);
        window.render_frame(cx);
        assert!(window.try_find("appearance-opacity-row").is_some());
        assert!(window.try_find("appearance-ui-font-row").is_none());
        window.click("settings-search", cx);
        window.input("UI 字体", cx);
    })
    .expect("navigation");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("appearance-ui-font-row").is_some());
        assert!(window.try_find("appearance-opacity-row").is_none());
        window.press("ctrl-a", cx);
        window.input("no-matching-setting", cx);
    })
    .expect("search all groups");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-no-results").visible());
        window.click("settings-fonts", cx);
    })
    .expect("empty results");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).search.read(cx).value().is_empty());
        assert!(window.try_find("appearance-ui-font-row").is_some());
    })
    .expect("navigation resets search");
}

#[gpui_kit::test]
fn compact_settings_actions_render_and_persist_without_save(cx: &mut TestAppContext) {
    use crate::theme::preferences::Preferences;
    use std::{cell::Cell, rc::Rc};

    let directory = tempfile::tempdir().expect("settings fixture");
    let path = directory.path().join("appearance.json");
    std::fs::write(
        &path,
        r#"{"theme_source":"builtin","content_opacity":0.7,"content_line_height":1.8}"#,
    )
    .expect("fixture preferences");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(None, Some(path.clone()), cx);
    });
    let closed = Rc::new(Cell::new(false));
    let observed = closed.clone();
    let window = cx.open_window(size(px(680.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        cx.subscribe(&settings, move |_, _, _: &Close, _| observed.set(true))
            .detach();
        Root::new(settings, window, cx)
    });
    let read = || {
        Preferences::from_json(&std::fs::read_to_string(&path).expect("saved fixture"))
            .expect("valid preferences")
    };
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-layout", cx);
        window.render_frame(cx);
        for id in [
            "appearance-opacity-default",
            "appearance-line-height-default",
            "settings-reset",
            "settings-close",
        ] {
            let button = window.find(id);
            assert!(button.visible(), "{id}");
            assert_eq!(button.role(), Some(gpui_kit::Role::Button));
            let bounds = button.bounds();
            assert!(bounds.origin.x >= px(0.), "{id}");
            assert!(bounds.right() <= px(680.), "{id}");
            assert!(bounds.bottom() <= px(720.), "{id}");
            assert_eq!(bounds.size.height, window.rem_size() * 1.5, "{id}");
        }
        let input = window.find("appearance-opacity").bounds();
        let default = window.find("appearance-opacity-default").bounds();
        assert!(input.size.width > px(80.));
        assert!(input.right() <= default.origin.x);
        window.hover("appearance-opacity-default", cx);
        window.render_frame(cx);
        window.click("appearance-opacity-default", cx);
    })
    .expect("compact layout and row reset");
    assert_eq!(read().content_opacity, None);
    assert_eq!(read().content_line_height, Some(1.8));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-reset", cx);
    })
    .expect("reset all");
    assert_eq!(read(), Preferences::default());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-close", cx);
    })
    .expect("close action");
    assert!(closed.get());
}

#[gpui_kit::test]
fn unreadable_preferences_keep_reset_visible_but_inert(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings fixture");
    let path = directory.path().join("appearance.json");
    std::fs::write(&path, "invalid fixture").expect("invalid preferences");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(None, Some(path.clone()), cx);
    });
    let window = cx.open_window(size(px(680.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        Root::new(settings, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let reset = window.find("settings-reset");
        assert!(reset.visible());
        assert_eq!(reset.label(), Some("恢复全部默认"));
        window.hover("settings-reset", cx);
        window.render_frame(cx);
        window.click("settings-reset", cx);
    })
    .expect("disabled reset");
    assert_eq!(
        std::fs::read_to_string(path).expect("unchanged invalid fixture"),
        "invalid fixture"
    );
}
