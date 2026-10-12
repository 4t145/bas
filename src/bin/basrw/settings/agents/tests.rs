use super::*;
use gpui_kit::{ScrollDelta, TestAppContext, point, px, size, test::TestWindowExt};

/// Reach controls with native wheel events, without bypassing viewport hit testing.
fn click(window: &mut Window, id: &'static str, cx: &mut App) {
    for _ in 0..24 {
        window.render_frame(cx);
        let target = window.find(id);
        let bounds = target.bounds();
        let viewport = window.find("agents-sections").bounds();
        if target.visible()
            && ((bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom())
                || id == "agents-reload")
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
    window.input(value, cx);
}

#[gpui_kit::test]
fn custom_acp_save_preserves_other_settings(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("temporary settings directory");
    let path = directory.path().join("settings.json");
    let initial = serde_json::json!({
        "appearance": {"theme_source": "builtin"},
        "daemon": {"agents": [{
            "id": "embedded", "kind": "embedded", "env": {"KEEP": "original"},
            "cancellation_grace_ms": 5000
        }]}
    });
    std::fs::write(
        &path,
        serde_json::to_vec(&initial).expect("serialize fixture"),
    )
    .expect("write initial settings");
    let initial_settings: basr::config::Settings =
        serde_json::from_value(initial.clone()).expect("typed initial settings");
    cx.update(gpui_kit::init);
    let mut view = None;
    let window = cx.open_window(size(px(760.), px(680.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(Store::at(path.clone()), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("created Agents view");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for id in [
            "agents-preset-codex",
            "agents-preset-opencode",
            "agents-preset-pi",
        ] {
            assert!(window.try_find(id).is_none(), "no preset shortcut remains");
        }
        click(window, "agents-add-custom", cx);
        window.render_frame(cx);
        assert!(view.read(cx).dirty(cx));
        assert!(
            view.read(cx).candidate(cx).is_err(),
            "custom path and ID are required"
        );
        assert!(
            window.try_find("agents-grace").is_none(),
            "advanced settings require a connection"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).expect("unchanged"))
                .expect("json"),
            initial
        );
        click(window, "agents-cancel", cx);
    })
    .expect("custom draft without writes");
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-add-custom", cx);
        fill(window, "agents-id", "custom", cx);
        fill(window, "agents-program", "/test/agent", cx);
        fill(
            window,
            "agents-args",
            "[\"a b\", \"\", \"--literal=$HOME\"]",
            cx,
        );
        click(window, "agents-save", cx);
    })
    .expect("save arguments through UI");
    let saved = Store::at(path.clone())
        .load()
        .expect("load saved arguments");
    assert_eq!(
        saved.settings.daemon.agents[1]
            .command
            .as_ref()
            .expect("saved ACP command")
            .args,
        ["a b", "", "--literal=$HOME"]
    );
    assert_eq!(saved.settings.daemon.agents[0].env["KEEP"], "original");
    assert_eq!(saved.settings.appearance, initial_settings.appearance);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-edit-1", cx);
        click(window, "agents-delete", cx);
        window.render_frame(cx);
        assert_eq!(window.find("agents-save").label(), Some("移除配置"));
        // Deletion, like command editing, requires explicit saving.
        assert_eq!(
            Store::at(path.clone())
                .load()
                .expect("read before deletion save")
                .settings
                .daemon
                .agents
                .len(),
            2
        );
        click(window, "agents-save", cx);
    })
    .expect("delete selected agent through UI");
    let saved = Store::at(path.clone()).load().expect("load saved deletion");
    assert_eq!(saved.settings.daemon.agents.len(), 1);
    assert_eq!(saved.settings.daemon.agents[0].env["KEEP"], "original");
    assert_eq!(saved.settings.appearance, initial_settings.appearance);
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
        .save_agents(&mut document, vec![AgentSpec::embedded(), spec])
        .expect("write initial agent catalog");
    cx.update(gpui_kit::init);
    let mut view = None;
    let window = cx.open_window(size(px(760.), px(680.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(Store::at(path.clone()), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("created Agents view");
    let before = std::fs::read(&path).expect("read initial catalog bytes");
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-add-custom", cx);
        click(window, "agents-save", cx);
        assert_eq!(
            std::fs::read(&path).expect("read rejected-save bytes"),
            before
        );
        assert!(view.read(cx).dirty(cx));
        click(window, "agents-cancel", cx);
        click(window, "agents-edit-1", cx);
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
            .env["KEEP"],
        "original"
    );
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-add-custom", cx);
        fill(window, "agents-id", "other", cx);
        fill(window, "agents-program", "/test/other", cx);
        let mut external = store.load().expect("load independent external document");
        store
            .save_agents(&mut external, vec![AgentSpec::embedded()])
            .expect("persist external catalog modification");
        let external_bytes = std::fs::read(&path).expect("read external catalog bytes");
        click(window, "agents-save", cx);
        assert!(view.read(cx).dirty(cx));
        assert_eq!(
            std::fs::read(&path).expect("read after conflicting save"),
            external_bytes
        );
        click(window, "agents-reload", cx);
        assert!(view.read(cx).dirty(cx));
        click(window, "agents-cancel", cx);
        click(window, "agents-reload", cx);
        click(window, "agents-edit-0", cx);
        click(window, "agents-delete", cx);
        click(window, "agents-save", cx);
        assert_eq!(
            std::fs::read(&path).expect("read rejected empty catalog bytes"),
            external_bytes
        );
    })
    .expect("preserve conflict draft and reject empty catalog through UI");
}

#[gpui_kit::test]
fn only_matching_connected_specs_unlock_advanced_fields(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("agent configuration");
    let store = Store::at(directory.path().join("settings.json"));
    let spec = AgentSpec::acp("test", Command::new("/test/adapter", ["acp".into()]));
    let mut document = store.load().expect("document");
    store
        .save_agents(&mut document, vec![spec.clone()])
        .expect("catalog");
    cx.update(gpui_kit::init);
    let mut page = None;
    let window = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let view = cx.new(|cx| Agents::new(store, window, cx));
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    let page = page.expect("view");
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| page.edit(spec.clone(), Some(0), window, cx));
        window.render_frame(cx);
        assert!(
            window.try_find("agents-program").is_some(),
            "path stays editable while disconnected"
        );
        assert!(window.try_find("agents-grace").is_none());
        page.update(cx, |page, cx| {
            page.apply_check(&spec, check::Status::Installed("/test/adapter".into()));
            assert!(
                !page.draft_connected(cx),
                "finding a file is not a handshake"
            );
            page.apply_check(&spec, check::Status::Connected("/test/adapter".into()));
            assert!(page.draft_connected(cx));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("agents-grace").is_some());
        page.update(cx, |page, cx| {
            page.form
                .program
                .update(cx, |input, cx| input.set_value("/test/changed", window, cx));
            page.form
                .grace
                .update(cx, |input, cx| input.set_value("777", window, cx));
            assert!(
                !page.draft_connected(cx),
                "changing the path invalidates readiness"
            );
            let candidate = page.candidate(cx).expect("valid connection draft");
            assert_eq!(
                candidate[0].cancellation_grace, spec.cancellation_grace,
                "hidden advanced edits cannot bypass connection gating"
            );
            let check = page.checks.get_mut(&spec.id).expect("check");
            check.spec = candidate[0].clone();
            check.status = check::Status::Checking;
            page.apply_check(&spec, check::Status::Connected("/test/adapter".into()));
            assert!(
                page.status(&spec).is_none(),
                "late response cannot verify a different config"
            );
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("agents-grace").is_none());
    })
    .expect("connection gated configuration");
}
