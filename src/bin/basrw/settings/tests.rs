use super::*;
use gpui_kit::{TestAppContext, size, test::TestWindowExt};

#[gpui_kit::test]
fn navigation_labels_fill_the_left_edge_for_categories_and_sections(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings fixture");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(None, Some(directory.path().join("settings.json")), cx);
    });
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        Root::new(cx.new(|cx| Settings::new(window, cx)), window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for id in [
            "settings-appearance",
            "settings-theme",
            "settings-fonts",
            "settings-layout",
            "settings-agents",
            "settings-agent-0",
            "settings-agents-add",
        ] {
            let button = window.find(id);
            let text = window.find(format!("{id}-text")).bounds();
            let bounds = button.bounds();
            assert_eq!(button.role(), Some(gpui_kit::Role::Button), "{id}");
            assert!(button.label().is_some(), "native accessible name for {id}");
            // A full-width text child prevents Button's internal centering
            // from moving shorter labels to a different start position.
            assert!(text.left() - bounds.left() <= window.rem_size(), "{id}");
            assert!(text.size.width > bounds.size.width / 2., "{id}");
        }
        let theme = window.find("settings-theme-text").bounds();
        let fonts = window.find("settings-fonts-text").bounds();
        assert_eq!(theme.left(), fonts.left());
    })
    .expect("left aligned navigation labels");
}

#[gpui_kit::test]
fn categories_are_nested_and_agent_drafts_survive_navigation(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("isolated unified settings");
    let path = directory.path().join("settings.json");
    let initial = r#"{"appearance":{"theme_source":"builtin","content_opacity":0.75}}"#;
    std::fs::write(&path, initial).expect("initial settings");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(None, Some(path.clone()), cx);
    });
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        Root::new(settings, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let parent = window.find("settings-appearance").bounds();
        for id in ["settings-theme", "settings-fonts", "settings-layout"] {
            let child = window.find(id).bounds();
            assert!(child.left() > parent.left(), "children are indented");
            assert!(child.top() >= parent.bottom());
        }
        let fonts = window.find("appearance-fonts-card").bounds();
        window.click("settings-fonts", cx);
        window.render_frame(cx);
        assert!(
            window.find("appearance-fonts-card").bounds().top() < fonts.top(),
            "a section entry scrolls the page instead of replacing it"
        );
        window.click("settings-appearance-expand", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-fonts").is_none());
        assert!(
            window.try_find("appearance-fonts-card").is_some(),
            "collapsing only hides navigation"
        );
        window.click("settings-appearance-expand", cx);
        window.render_frame(cx);
        assert!(window.find("settings-fonts").visible());
        assert_eq!(
            std::fs::read_to_string(&path).expect("unchanged file"),
            initial
        );
        window.click("settings-agents", cx);
        window.render_frame(cx);
        assert!(window.find("agents-page").visible());
        assert!(
            window.find("settings-agent-0").visible(),
            "configured agents are listed as sections"
        );
        window.click("settings-agents-add", cx);
        window.render_frame(cx);
        window.click("agents-add-custom", cx);
        window.render_frame(cx);
        window.click("settings-agents-add", cx);
        window.render_frame(cx);
        window.click("agents-id", cx);
        window.input("codex", cx);
        window.click("agents-program", cx);
        window.press("ctrl-a", cx);
        window.input("/test/codex-acp", cx);
        window.click("settings-layout", cx);
        window.render_frame(cx);
        assert!(window.find("appearance-opacity-row").visible());
        window.click("settings-search", cx);
        window.input("codex", cx);
    })
    .expect("tree and page navigation");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("agents-page").visible(),
            "search finds the agent category"
        );
        window.click("agents-save", cx);
    })
    .expect("save the retained agent draft");
    let settings = basr::config::Store::at(path)
        .load()
        .expect("saved unified config")
        .settings;
    assert_eq!(settings.appearance.content_opacity, Some(0.75));
    assert_eq!(settings.daemon.agents.len(), 2);
    assert_eq!(
        settings.daemon.agents[1]
            .command
            .as_ref()
            .expect("ACP command")
            .program,
        std::path::PathBuf::from("/test/codex-acp")
    );
}

#[gpui_kit::test]
fn navigation_reveals_sections_and_search_filters_settings(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings config");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, r#"{"appearance":{"theme_source":"builtin"}}"#).expect("builtin config");
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
        let fonts = window.find("appearance-fonts-card").bounds();
        window.click("settings-layout", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("appearance-ui-font-row").is_some(),
            "sections share one page instead of replacing each other"
        );
        assert!(window.find("appearance-layout-card").bounds().top() < fonts.top());
        window.click("settings-search", cx);
        window.input("界面字体", cx);
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
    let path = directory.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"appearance":{"theme_source":"builtin","content_opacity":0.7,"content_line_height":1.8}}"#,
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
        basr::config::Store::at(path.clone())
            .load()
            .expect("valid settings")
            .settings
            .appearance
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
            assert_eq!(bounds.size.height, px(crate::ui::CONTROL), "{id}");
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
    let path = directory.path().join("settings.json");
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
        assert_eq!(reset.label(), Some("重置外观设置"));
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
