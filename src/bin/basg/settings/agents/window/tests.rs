use super::*;
use gpui_kit::{TestAppContext, component::Root, test::TestWindowExt};

#[gpui_kit::test]
fn native_installation_is_separate_reused_and_close_cancels_only_new_drafts(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("settings");
    let path = directory.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"daemon":{"agents":[{"id":"custom","kind":"acp","command":{"program":"/fixture/acp","args":[]}}]}}"#,
    )
    .expect("fixture");
    let before = std::fs::read(&path).expect("original bytes");
    cx.update(super::super::tests::init);
    let mut agents = None;
    let configuration = cx.open_window(size(px(900.), px(720.)), |window, cx| {
        let view = cx.new(|cx| Agents::new(bas::config::Store::at(path.clone()), window, cx));
        agents = Some(view.clone());
        Root::new(view, window, cx)
    });
    let agents = agents.expect("agents");
    cx.update_window(configuration.into(), |_, window, cx| {
        window.render_frame(cx);
        for id in [
            "agents-add-custom",
            "agents-card-add",
            "agents-registry-search",
            "agents-start-chat-0",
        ] {
            assert!(
                window.try_find(id).is_none(),
                "{id} does not belong to configuration"
            );
        }
        assert_eq!(agents.read(cx).sections().len(), 1);
    })
    .expect("configuration surface");
    let installation = cx.update(|cx| open(agents.clone(), cx).expect("native installation"));
    let again = cx.update(|cx| open(agents.clone(), cx).expect("reuse installation"));
    assert_ne!(configuration.window_id(), installation.window_id());
    assert_eq!(installation.window_id(), again.window_id());
    cx.update_window(installation.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("agents-add-custom", cx);
        window.render_frame(cx);
        window.click("agents-id", cx);
        window.input("unsaved", cx);
        assert!(agents.read(cx).dirty(cx));
    })
    .expect("native custom draft");
    cx.update_window(configuration.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("agents-id").is_none(),
            "new draft never becomes a configuration editor"
        );
        assert!(window.find("agents-edit-0").visible());
    })
    .expect("configuration stays configuration");
    cx.update_window(installation.into(), |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
    })
    .expect("close native window");
    cx.update(|cx| {
        assert!(!agents.read(cx).dirty(cx));
        assert_eq!(std::fs::read(&path).expect("unchanged disk"), before);
    });
    let reopened = cx.update(|cx| open(agents.clone(), cx).expect("reopen"));
    assert_ne!(installation.window_id(), reopened.window_id());
    cx.update_window(configuration.into(), |_, window, cx| {
        window.click("agents-edit-0", cx);
        window.render_frame(cx);
        window.click("agents-id", cx);
        window.press("ctrl-a", cx);
        window.input("edited", cx);
        assert!(agents.read(cx).dirty(cx));
    })
    .expect("existing configuration draft");
    cx.update_window(reopened.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agents-id").is_none());
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
    })
    .expect("closing installation preserves configuration edit");
    cx.update(|cx| assert!(agents.read(cx).dirty(cx)));
}

#[gpui_kit::test]
fn catalog_can_open_before_settings_and_uses_the_same_agents_owner(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let path = directory.path().join("settings.json");
    cx.update(|cx| {
        super::super::tests::init(cx);
        crate::configuration::init(Some(path), cx);
    });
    let native = cx.update(|cx| open_catalog(cx).expect("catalog without settings"));
    cx.update(|cx| {
        assert_eq!(cx.windows().len(), 1, "catalog does not open Settings");
    });
    cx.update_window(native.into(), |_, window, cx| {
        let first = super::super::shared(window, cx);
        let second = super::super::shared(window, cx);
        assert_eq!(first.entity_id(), second.entity_id());
        assert_eq!(
            open(second, cx).expect("reuse").window_id(),
            native.window_id()
        );
    })
    .expect("shared owner");
}
