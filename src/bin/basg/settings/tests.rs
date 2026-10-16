use super::*;
use gpui_kit::{TestAppContext, size, test::TestWindowExt};

#[gpui_kit::test]
fn shared_header_tracks_configuration_ownership(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("isolated frontend settings");
    let path = directory.path().join("frontend.json");
    std::fs::write(&path, r#"{"appearance":{"theme_source":"builtin"}}"#)
        .expect("frontend configuration");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        crate::theme::init(None, cx);
    });
    let mut settings = None;
    let window = cx.open_window(size(px(680.), px(720.)), |window, cx| {
        let view = cx.new(|cx| Settings::new(window, cx));
        settings = Some(view.clone());
        crate::theme::window_root(view, window, cx)
    });
    let settings = settings.expect("settings view");
    let mut file_bounds = None;
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(settings.read(cx).settings_path(cx), Some(path.clone()));
        let file = window.find("settings-edit-file");
        assert!(file.visible());
        assert!(settings.read(cx).can_edit_file(cx));
        file_bounds = Some(file.bounds());
        assert!(window.try_find("settings-backend-context").is_none());
        assert_eq!(
            window.find("settings-frontend-context").label(),
            Some("本机设置")
        );
        assert!(window.try_find("settings-scope-frontend").is_none());
        assert!(window.try_find("settings-scope-backend").is_none());
        assert!(window.try_find("settings-add-agent").is_none());
        window.click("settings-agents", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-frontend-context").is_none());
        let backend_context = crate::i18n::tr!(
            "settings-backend-context",
            address = crate::i18n::tr!("settings-backend-disconnected")
        );
        assert_eq!(
            window.find("settings-backend-context").label(),
            Some(backend_context.as_str())
        );
        assert!(settings.read(cx).settings_path(cx).is_none());
        let file = window.find("settings-edit-file");
        assert!(!settings.read(cx).can_edit_file(cx));
        assert_eq!(Some(file.bounds()), file_bounds);
        assert!(window.try_find("agents-card-add").is_none());
        assert!(window.try_find("agents-installation-help").is_none());
        assert!(window.try_find("settings-agents-add").is_none());
        assert!(window.try_find("settings-add-agent").is_none());
        let navigation = window.find("settings-navigation").bounds();
        let add = window.find("agents-add").bounds();
        let actions = window.find("agents-configuration-actions").bounds();
        let chapter = window.find("agents-configuration-header").bounds();
        let global = window.find("settings-header").bounds();
        assert!(add.left() >= navigation.right());
        assert!(add.left() >= actions.left() && add.right() <= actions.right());
        assert!(add.top() >= chapter.top() && add.bottom() <= chapter.bottom());
        assert!(chapter.top() >= global.bottom());
        window.click("settings-appearance", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-backend-context").is_none());
        assert_eq!(
            window.find("settings-frontend-context").label(),
            Some("本机设置")
        );
        assert_eq!(settings.read(cx).settings_path(cx), Some(path.clone()));
        assert!(settings.read(cx).can_edit_file(cx));
        assert_eq!(
            Some(window.find("settings-edit-file").bounds()),
            file_bounds
        );
        std::fs::remove_file(&path).expect("remove the test-only settings file");
        window.render_frame(cx);
        assert!(!settings.read(cx).can_edit_file(cx));
    })
    .expect("one header and file entry across configuration scopes");
}

#[gpui_kit::test]
fn navigation_labels_fill_the_left_edge_for_categories_and_sections(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings fixture");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(directory.path().join("settings.json")), cx);
        crate::theme::init(None, cx);
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
        crate::configuration::init(Some(path.clone()), cx);
        crate::theme::init(None, cx);
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
        assert!(window.try_find("agents-card-add").is_none());
        assert!(window.try_find("agents-registry").is_none());
        window.click("agents-add", cx);
    })
    .expect("agent configuration is separate from onboarding");
    let onboarding = cx.update(|cx| agents::window::open_catalog(cx).expect("agent onboarding"));
    assert_ne!(window.window_id(), onboarding.window_id());
    cx.update_window(onboarding.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("agents-add-custom", cx);
        window.render_frame(cx);
        window.click("agents-id", cx);
        window.input("codex", cx);
        window.click("agents-program", cx);
        window.press("ctrl-a", cx);
        window.input("/test/codex-acp", cx);
    })
    .expect("edit a draft in the separate onboarding window");
    cx.update_window(window.into(), |_, window, cx| {
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
        assert!(window.try_find("agents-save").is_none());
    })
    .expect("configuration does not host the add-agent form");
    cx.update_window(onboarding.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("agents-save", cx);
    })
    .expect("save the retained agent draft");
    let settings = bas::config::Store::at(path)
        .load()
        .expect("saved unified config")
        .settings;
    assert_eq!(settings.appearance.content_opacity, Some(0.75));
    assert_eq!(settings.daemon.agents.len(), 2);
    assert_eq!(
        settings.daemon.agents[1]
            .as_custom()
            .expect("custom registration")
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
        crate::configuration::init(Some(path), cx);
        crate::theme::init(None, cx);
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

    let directory = tempfile::tempdir().expect("settings fixture");
    let path = directory.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"appearance":{"theme_source":"builtin","content_opacity":0.7,"content_line_height":1.8}}"#,
    )
    .expect("fixture preferences");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        crate::theme::init(None, cx);
    });
    let window = cx.open_window(size(px(680.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        Root::new(settings, window, cx)
    });
    let read = || {
        bas::config::Store::at(path.clone())
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
        ] {
            let button = window.find(id);
            assert!(button.visible(), "{id}");
            assert_eq!(button.role(), Some(gpui_kit::Role::Button));
            let bounds = button.bounds();
            assert!(bounds.origin.x >= px(0.), "{id}");
            assert!(bounds.right() <= px(680.), "{id}");
            assert!(bounds.bottom() <= px(720.), "{id}");
            let height = if id == "settings-reset" {
                crate::ui::CONTROL
            } else {
                crate::ui::COMPACT_CONTROL
            };
            assert_eq!(bounds.size.height, px(height), "{id}");
        }
        let input = window.find("appearance-opacity").bounds();
        let default = window.find("appearance-opacity-default").bounds();
        assert!(input.size.width > px(crate::ui::COMPACT_CONTROL));
        assert!(default.right() <= window.find("appearance-opacity-stepper").bounds().left());
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
        assert!(window.try_find("settings-close").is_none());
    })
    .expect("no duplicate close action");
}

#[gpui_kit::test]
fn unreadable_preferences_keep_reset_visible_but_inert(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings fixture");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, "invalid fixture").expect("invalid preferences");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        crate::theme::init(None, cx);
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
