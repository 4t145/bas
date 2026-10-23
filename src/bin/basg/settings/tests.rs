use super::*;
use bas::{config::ConnectionProfile, service::Target};
use gpui_kit::{TestAppContext, component::Root, size, test::TestWindowExt};

#[gpui_kit::test]
fn settings_group_by_owner_and_keep_conversation_behavior_out_of_appearance_reset(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("settings");
    let path = directory.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"appearance":{"ui_font_size":17,"composer_enter_behavior":"newline"}}"#,
    )
    .expect("frontend preferences");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
    });
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        Root::new(settings, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-frontend-group").visible());
        assert!(window.find("settings-backend-group").visible());
        assert_eq!(
            window.find("settings-session-management").label(),
            Some("会话管理")
        );
        assert!(
            window.find("settings-session-management").bounds().origin.y
                < window.find("settings-agents").bounds().origin.y
        );
        window.click("settings-sessions", cx);
        window.render_frame(cx);
        assert!(window.find("appearance-composer-enter").visible());
        window.click("settings-appearance", cx);
        window.render_frame(cx);
        assert!(window.try_find("appearance-composer-enter").is_none());
        window.click("settings-reset", cx);
    })
    .expect("logical settings arrangement");
    let saved = bas::config::FrontendStore::at(path)
        .load()
        .expect("saved frontend");
    assert_eq!(
        saved.settings.appearance.composer_enter_behavior,
        bas::config::appearance::ComposerEnterBehavior::Newline
    );
    assert_eq!(saved.settings.appearance.ui_font_size, None);
    assert!(matches!(
        Category::for_query(&search::Query::new("会话管理")),
        Some(Category::SessionManagement)
    ));
    assert!(matches!(
        Category::for_query(&search::Query::new("预热")),
        Some(Category::SessionManagement)
    ));
    assert!(matches!(
        Category::for_query(&search::Query::new("Enter")),
        Some(Category::Sessions)
    ));
}

fn check_footer_without_copy(cx: &mut TestAppContext, message: String) {
    let directory = tempfile::tempdir().expect("settings directory");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, "{}").expect("settings fixture");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
        crate::agents::init(Target::Local.into(), runtime.handle().clone(), cx);
    });
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| {
            let mut settings = Settings::new(window, cx);
            settings.connection_message = message.clone();
            settings
        });
        Root::new(settings, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-save-status").visible());
        assert!(window.try_find("settings-save-status-copy").is_none());
        assert_eq!(
            window.find("settings-save-status-message").label(),
            Some(message.as_str()),
        );
        assert!(window.find("settings-connections").visible());
    })
    .expect("settings feedback window");
}

#[gpui_kit::test]
fn settings_footer_preserves_feedback_without_copy_action(cx: &mut TestAppContext) {
    check_footer_without_copy(
        cx,
        format!(
            "Connection response\n{}\n trailing space ",
            "endpoint/".repeat(80)
        ),
    );
}

#[gpui_kit::test]
fn empty_settings_footer_has_no_orphan_copy_button(cx: &mut TestAppContext) {
    check_footer_without_copy(cx, String::new());
}

#[gpui_kit::test]
fn search_events_route_and_filter_with_the_same_metadata(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings directory");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, "{}").expect("settings fixture");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
    });
    let mut entity = None;
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        entity = Some(settings.clone());
        Root::new(settings, window, cx)
    });
    let entity = entity.expect("settings entity");
    cx.update_window(window.into(), |_, window, cx| {
        for (current, query, expected) in [
            (Category::Agents, "Fonts", Category::Appearance),
            (Category::Appearance, "Enter", Category::Sessions),
            (Category::Sessions, " \t ", Category::Sessions),
            (
                Category::Connections,
                "no-such-setting-123",
                Category::Appearance,
            ),
            (Category::Agents, "no-such-setting-123", Category::Agents),
        ] {
            entity.update(cx, |settings, cx| {
                settings.category = current;
                let input = settings.search.clone();
                input.update(cx, |input, cx| input.set_value(query, window, cx));
                settings.on_search(&input, &InputEvent::Change, window, cx);
                assert_eq!(settings.category, expected, "{query}");
                assert_eq!(settings.appearance.read(cx).query, query);
                assert_eq!(settings.agents.read(cx).query, query);
            });
            window.render_frame(cx);
            match query {
                "Fonts" => {
                    assert!(window.find("appearance-fonts-card").visible());
                    assert!(window.try_find("appearance-theme-card").is_none());
                }
                "Enter" => assert!(window.find("appearance-composer-enter").visible()),
                "no-such-setting-123" if expected == Category::Appearance => {
                    assert!(window.find("settings-no-results").visible());
                }
                _ => {}
            }
        }
    })
    .expect("search navigation and page filtering");
}

#[gpui_kit::test]
fn frontend_connections_and_backend_switching_are_distinct_settings_scopes(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    std::fs::write(&path, "{}").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
        crate::agents::init(Target::Local.into(), runtime.handle().clone(), cx);
    });
    let mut entity = None;
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let settings = cx.new(|cx| Settings::new(window, cx));
        entity = Some(settings.clone());
        Root::new(settings, window, cx)
    });
    let entity = entity.unwrap();
    let old_agents = cx.update(|cx| entity.read(cx).agents.clone());
    let old_policy = cx.update(|cx| entity.read(cx).acp.entity_id());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-connections", cx);
        window.render_frame(cx);
        assert!(window.find("connections-page").visible());
        assert!(window.find("settings-frontend-context").visible());
        entity.update(cx, |settings, cx| {
            settings
                .profiles
                .update(cx, |profiles, cx| {
                    profiles.put(
                        ConnectionProfile {
                            id: "second".into(),
                            name: "Second server".into(),
                            note: None,
                            target: Target::Pipe("bas-other".into()),
                            tls: None,
                        },
                        cx,
                    )
                })
                .unwrap();
            settings.choose_backend("second", cx);
            settings.select_category(Category::Agents, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-backend-selector").visible());
        assert!(window.try_find("settings-open-backend").is_none());
        assert!(window.find("settings-edit-file").visible());
        assert!(window.find("settings-backend-context").visible());
        assert_eq!(
            crate::connections::current(cx),
            Some(Target::Pipe("bas-other".into()).into())
        );
        assert_ne!(entity.read(cx).agents.entity_id(), old_agents.entity_id());
        assert_ne!(entity.read(cx).acp.entity_id(), old_policy);
        window.click("settings-session-management", cx);
        window.render_frame(cx);
        assert!(window.find("acp-policy-page").visible());
        assert!(window.find("settings-backend-context").visible());
        assert!(window.find("settings-backend-selector").visible());
        entity.update(cx, |settings, cx| settings.choose_backend("local", cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(entity.read(cx).agents.entity_id(), old_agents.entity_id());
        assert_eq!(entity.read(cx).acp.entity_id(), old_policy);
        assert_eq!(crate::connections::current(cx), Some(Target::Local.into()));
        crate::connections::manage(Target::Pipe("bas-other".into()).into(), cx);
        window::open_agent_management("embedded", Some(Target::Local.into()), cx)
            .expect("manage action retains its originating backend");
        assert_eq!(crate::connections::current(cx), Some(Target::Local.into()));
    });
}
