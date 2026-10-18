use super::*;
use gpui_kit::{TestAppContext, test::TestWindowExt};

#[gpui_kit::test]
fn custom_save_distinguishes_persistence_from_connection_and_links_to_management(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("settings");
    let path = directory.path().join("settings.json");
    let initial = r#"{"appearance":{"theme_source":"builtin"}}"#;
    std::fs::write(&path, initial).expect("isolated settings");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        crate::theme::init(None, cx);
    });
    let installation = cx.update(|cx| window::open_catalog(cx).expect("add window"));
    let agents = cx
        .update_window(installation.into(), |_, window, cx| {
            let agents = shared(window, cx);
            window.render_frame(cx);
            window.click("agents-add-custom", cx);
            window.render_frame(cx);
            assert_eq!(window.find("agents-save").label(), Some("保存"));
            window.click("agents-save", cx);
            window.render_frame(cx);
            assert!(window.find("agents-form-message").visible());
            assert!(window.find("agents-id").visible());
            assert_eq!(
                std::fs::read_to_string(&path).expect("unchanged settings"),
                initial,
                "invalid drafts never become saved configurations"
            );
            for (id, value) in [("agents-id", "custom"), ("agents-program", "/fixture/acp")] {
                window.click(id, cx);
                window.press("ctrl-a", cx);
                window.input(value, cx);
            }
            window.click("agents-save", cx);
            agents.update(cx, |agents, cx| {
                let saved = agents.saved.clone().expect("saved configuration");
                agents.apply_check(&saved, Status::Connecting);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.try_find("agents-id").is_none());
            assert_eq!(
                window.find("agents-save-result").label(),
                Some(crate::i18n::tr!("agent-saved-connecting", agent = "custom").as_str())
            );
            agents
        })
        .expect("save from the native add window");
    let saved_bytes = std::fs::read(&path).expect("persisted before connection completes");
    cx.update_window(installation.into(), |_, window, cx| {
        agents.update(cx, |agents, cx| {
            let saved = agents.saved.clone().expect("saved configuration");
            agents.apply_check(&saved, Status::Failed("Agent connection timed out".into()));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("agents-save-error").visible());
        assert_eq!(
            window.find("agents-save-result").label(),
            Some(crate::i18n::tr!("agent-saved-unavailable", agent = "custom").as_str())
        );
        assert_eq!(std::fs::read(&path).expect("still saved"), saved_bytes);
        window.click("agents-saved-manage", cx);
    })
    .expect("connection failure stays visible with a management entry");
    cx.run_until_parked();
    let settings = cx.update(|cx| crate::settings::window::current(cx).expect("management window"));
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shared(window, cx).entity_id(), agents.entity_id());
        assert_eq!(window.find("agents-source-custom").label(), Some("自定义"));
        assert!(window.find("agents-item-custom").visible());
        assert!(
            matches!(agents.read(cx).status(agents.read(cx).saved.as_ref().expect("saved")),
            Some(Status::Failed(message)) if message == "Agent connection timed out"),
            "opening management does not silently start another probe"
        );
        window.click("agents-edit-0", cx);
        window.render_frame(cx);
        assert_eq!(agents.read(cx).form.values(cx).program, "/fixture/acp");
        assert!(window.try_find("agents-delete").is_none());
        let cancel = window.find("agents-cancel").bounds();
        let save = window.find("agents-save").bounds();
        assert_eq!(cancel.top(), save.top());
        assert!(cancel.right() < save.left());
    })
    .expect("manage the saved custom agent");
}
