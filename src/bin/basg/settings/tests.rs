use super::*;
use bas::{config::ConnectionProfile, service::Target};
use gpui_kit::{TestAppContext, component::Root, size, test::TestWindowExt};

#[gpui_kit::test]
fn settings_footer_copies_original_multiline_feedback(cx: &mut TestAppContext) {
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
    let message = format!(
        "Connection response\n{}\n trailing space ",
        "endpoint/".repeat(80)
    );
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
        window.click("settings-save-status-copy", cx);
        assert_eq!(
            cx.read_from_clipboard().expect("copied feedback").text(),
            Some(message.clone()),
        );
        assert!(window.find("settings-connections").visible());
    })
    .expect("settings feedback window");
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
        entity.update(cx, |settings, cx| settings.choose_backend("local", cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(entity.read(cx).agents.entity_id(), old_agents.entity_id());
        assert_eq!(crate::connections::current(cx), Some(Target::Local.into()));
        crate::connections::manage(Target::Pipe("bas-other".into()).into(), cx);
        window::open_agent_management("embedded", Some(Target::Local.into()), cx)
            .expect("manage action retains its originating backend");
        assert_eq!(crate::connections::current(cx), Some(Target::Local.into()));
    });
}
