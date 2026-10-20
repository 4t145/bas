use super::*;
use gpui_kit::{ScrollDelta, TestAppContext, point, px, size, test::TestWindowExt};

pub(super) fn init(cx: &mut App) {
    gpui_kit::init(cx);
    crate::theme::init_default(cx);
}

#[gpui_kit::test]
fn configuration_requests_disable_conflicting_editor_actions(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let store = Store::at(directory.path().join("settings.json"));
    let spec = AgentSpec::acp("custom", Command::new("/fixture/agent", []));
    let mut document = store.load().expect("document");
    store
        .save_agents(
            &mut document,
            vec![AgentSpec::embedded().into(), spec.clone().into()],
        )
        .expect("saved agents");
    let before = std::fs::read(store.path()).expect("original settings");
    cx.update(|cx| {
        init(cx);
        crate::application::init(cx);
    });
    let mut page = None;
    let window = cx.open_window(size(px(760.), px(680.)), |window, cx| {
        let view = cx.new(|cx| Agents::new(store.clone(), window, cx));
        page = Some(view.clone());
        gpui_kit::component::Root::new(view, window, cx)
    });
    let page = page.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.configuration_task =
                Some(cx.spawn(async |_, _| std::future::pending::<()>().await));
        });
        click(window, "agents-remove-custom", cx);
        assert_eq!(
            cx.windows().len(),
            1,
            "busy removal does not open confirmation"
        );
        assert_eq!(std::fs::read(store.path()).expect("settings"), before);
        page.update(cx, |page, cx| {
            page.configuration_task.take();
            page.edit(spec.clone(), Some(spec.id.clone()), window, cx);
            page.form.program.update(cx, |input, cx| {
                input.set_value("/fixture/changed", window, cx)
            });
            page.configuration_task =
                Some(cx.spawn(async |_, _| std::future::pending::<()>().await));
        });
        click(window, "agents-save", cx);
        click(window, "agents-cancel", cx);
        assert!(
            page.read(cx).dirty(cx),
            "pending save cannot discard the draft"
        );
        assert_eq!(std::fs::read(store.path()).expect("settings"), before);
        page.update(cx, |page, _| {
            page.configuration_task.take();
        });
        click(window, "agents-save", cx);
        assert!(!page.read(cx).dirty(cx));
        assert_ne!(std::fs::read(store.path()).expect("saved settings"), before);
    })
    .expect("configuration controls");
}

#[gpui_kit::test]
fn switching_management_preserves_backend_drafts_and_notifications(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let first: bas::service::tls::ConnectionOptions =
        "192.0.2.1:4554".parse().expect("first target");
    let second: bas::service::tls::ConnectionOptions =
        "192.0.2.2:4554".parse().expect("second target");
    cx.update(|cx| {
        init(cx);
        crate::configuration::init(Some(directory.path().join("settings.json")), cx);
        crate::agents::init(first.clone(), runtime.handle().clone(), cx);
    });
    cx.open_window(size(px(760.), px(680.)), |window, cx| {
        let original = shared(window, cx);
        original.update(cx, |page, cx| {
            page.form
                .id
                .update(cx, |input, cx| input.set_value("unsaved", window, cx));
            page.editor = Editor::Add(AgentSpec::acp("", Command::new("", [])));
            page.query = "original query".into();
        });
        crate::agents::init(second.clone(), runtime.handle().clone(), cx);
        let other = shared(window, cx);
        assert_ne!(original.entity_id(), other.entity_id());
        assert!(other.read(cx).query.is_empty());
        original.update(cx, |page, cx| {
            assert_eq!(page.target(), Some(first.clone()));
            assert_eq!(
                page.check_service.as_ref().expect("pinned check").address(),
                first
            );
            page.notify_agent_change(Some("ready".into()), cx);
        });
        assert_eq!(
            cx.global::<crate::agents::Changes>().server,
            Some(first.clone())
        );
        crate::agents::init(first.clone(), runtime.handle().clone(), cx);
        let restored = shared(window, cx);
        assert_eq!(original.entity_id(), restored.entity_id());
        assert_eq!(
            restored.read(cx).form.id.read(cx).value().to_string(),
            "unsaved"
        );
        assert_eq!(restored.read(cx).query, "original query");
        assert!(restored.read(cx).dirty(cx));
        gpui_kit::component::Root::new(original, window, cx)
    });
}

pub(super) fn managed_fixture(directory: &std::path::Path, spec: &AgentSpec) -> AgentRegistration {
    let path = directory.join(bas::config::INSTALLATION_FILE);
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "spec": spec,
    }))
    .expect("descriptor JSON");
    std::fs::write(&path, &bytes).expect("managed descriptor");
    AgentRegistration::Managed(bas::config::ManagedAgent {
        id: spec.id.clone(),
        installation: bas::config::InstallationRef::from_bytes(path, &bytes).expect("reference"),
    })
}

/// Reach controls with native wheel events, without bypassing viewport hit testing.
fn click(window: &mut Window, id: &'static str, cx: &mut App) {
    for _ in 0..24 {
        window.render_frame(cx);
        let target = window.find(id);
        let bounds = target.bounds();
        let viewport = window.find("agents-sections").bounds();
        if target.visible()
            && ((bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom())
                || id == "agents-reload"
                || id == "agents-add-custom")
        {
            window.click(id, cx);
            return;
        }
        let delta = if bounds.top() < viewport.top() {
            180.
        } else {
            -180.
        };
        window.scroll(
            "agents-sections",
            ScrollDelta::Pixels(point(px(0.), px(delta))),
            cx,
        );
    }
    panic!("control {id} was not reachable by scrolling the settings viewport");
}

fn fill(window: &mut Window, id: &'static str, value: &str, cx: &mut App) {
    click(window, id, cx);
    window.press("ctrl-a", cx);
    if value.is_empty() {
        window.press("backspace", cx);
    } else {
        window.input(value, cx);
    }
}

#[gpui_kit::test]
fn invalid_and_conflicting_edits_never_overwrite_disk(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("temporary settings directory");
    let path = directory.path().join("settings.json");
    let store = Store::at(path.clone());
    let mut document = store.load().expect("default settings for missing fixture");
    let mut spec = AgentSpec::acp("codex", Command::new("/test/codex-acp", []));
    spec.env.insert("KEEP".into(), "original".into());
    store
        .save_agents(
            &mut document,
            vec![AgentSpec::embedded().into(), spec.into()],
        )
        .expect("write initial agent catalog");
    cx.update(init);
    let mut view = None;
    let window = cx.open_window(size(px(760.), px(680.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(Store::at(path.clone()), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("created Agents view");
    let before = std::fs::read(&path).expect("read initial catalog bytes");
    let installation = cx.update(|cx| window::open(view.clone(), cx).expect("installation window"));
    cx.update_window(installation.into(), |_, window, cx| {
        click(window, "agents-add-custom", cx);
        click(window, "agents-save", cx);
        assert_eq!(
            std::fs::read(&path).expect("read rejected-save bytes"),
            before
        );
        assert!(view.read(cx).dirty(cx));
        click(window, "agents-cancel", cx);
    })
    .expect("reject invalid draft in installation window");
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-edit-0", cx);
        click(window, "agents-args", cx);
        window.press("ctrl-a", cx);
        window.input("[\"edited argument\"]", cx);
        click(window, "agents-save", cx);
    })
    .expect("reject invalid draft then edit existing agent through UI");
    assert_eq!(
        store
            .load()
            .expect("load edited agent")
            .settings
            .daemon
            .agents[1]
            .as_custom()
            .expect("custom registration")
            .env["KEEP"],
        "original"
    );
    cx.update_window(installation.into(), |_, window, cx| {
        click(window, "agents-add-custom", cx);
        fill(window, "agents-id", "other", cx);
        fill(window, "agents-program", "/test/other", cx);
        let mut external = store.load().expect("load independent external document");
        store
            .save_agents(&mut external, vec![AgentSpec::embedded().into()])
            .expect("persist external catalog modification");
        let external_bytes = std::fs::read(&path).expect("read external catalog bytes");
        click(window, "agents-save", cx);
        assert!(view.read(cx).dirty(cx));
        assert_eq!(
            std::fs::read(&path).expect("read after conflicting save"),
            external_bytes
        );
    })
    .expect("preserve conflicting installation draft");
    let external_bytes = std::fs::read(&path).expect("external catalog");
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-reload", cx);
        assert!(view.read(cx).dirty(cx));
    })
    .expect("reload does not discard installation draft");
    cx.update_window(installation.into(), |_, window, cx| {
        click(window, "agents-cancel", cx);
    })
    .expect("cancel installation draft");
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-reload", cx);
        window.render_frame(cx);
        assert!(window.try_find("agents-edit-0").is_none());
        assert!(window.try_find("agents-remove-embedded").is_none());
        assert_eq!(
            std::fs::read(&path).expect("embedded is read-only"),
            external_bytes
        );
    })
    .expect("preserve conflict draft and keep embedded read-only");
}

#[gpui_kit::test]
fn stale_custom_save_cannot_replace_a_new_managed_registration(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let path = directory.path().join("settings.json");
    let store = Store::at(path.clone());
    let spec = AgentSpec::acp("same-id", Command::new("/custom/acp", []));
    let managed = managed_fixture(directory.path(), &spec);
    let mut document = store.load().expect("document");
    store
        .save_agents(&mut document, vec![spec.clone().into()])
        .expect("custom");
    cx.update(gpui_kit::init);
    let mut view = None;
    let window = cx.open_window(size(px(960.), px(900.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(store.clone(), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |page, cx| {
            page.edit(spec.clone(), Some(spec.id.clone()), window, cx)
        });
        fill(window, "agents-program", "/custom/changed", cx);
        let mut external = store.load().expect("external document");
        store
            .save_agents(&mut external, vec![managed.clone()])
            .expect("managed replaces custom externally");
        click(window, "agents-save", cx);
        assert!(view.read(cx).dirty(cx));
        assert_eq!(
            store.load().expect("still managed").settings.daemon.agents,
            vec![managed.clone()]
        );
        click(window, "agents-cancel", cx);
        std::fs::remove_file(directory.path().join(bas::config::INSTALLATION_FILE))
            .expect("descriptor disappears");
        view.update(cx, |page, cx| page.reload(cx));
        window.render_frame(cx);
        assert!(window.find("agents-registration-errors").visible());
        assert!(
            view.read(cx).catalog().is_empty(),
            "no fallback custom agent"
        );
        assert_eq!(
            store
                .load()
                .expect("retained unresolved reference")
                .settings
                .daemon
                .agents,
            vec![managed.clone()]
        );
    })
    .expect("stale ownership and descriptor errors");
}
