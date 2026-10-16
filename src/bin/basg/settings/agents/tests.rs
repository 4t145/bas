use super::*;
use gpui_kit::{ScrollDelta, TestAppContext, point, px, size, test::TestWindowExt};

pub(super) fn init(cx: &mut App) {
    gpui_kit::init(cx);
    crate::theme::init_default(cx);
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
                || id == "agents-cancel")
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
    let initial_settings: bas::config::Settings =
        serde_json::from_value(initial.clone()).expect("typed initial settings");
    cx.update(init);
    let mut view = None;
    let window = cx.open_window(size(px(760.), px(680.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(Store::at(path.clone()), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("created Agents view");
    let installation = cx.update(|cx| window::open(view.clone(), cx).expect("installation window"));
    cx.update_window(installation.into(), |_, window, cx| {
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
            window.try_find("agents-delete").is_none(),
            "unsaved agents cannot be deleted"
        );
        let back = window.find("agents-cancel").bounds();
        let form = window.find("agents-id").bounds();
        let save = window.find("agents-save").bounds();
        let actions = window.find("agents-form-actions").bounds();
        assert!(back.bottom() < form.top());
        assert_eq!(
            back.size,
            size(
                px(crate::ui::COMPACT_CONTROL),
                px(crate::ui::COMPACT_CONTROL)
            )
        );
        assert_eq!(save.right(), actions.right());
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
    cx.update_window(installation.into(), |_, window, cx| {
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
            .as_custom()
            .expect("custom registration")
            .command
            .as_ref()
            .expect("saved ACP command")
            .args,
        ["a b", "", "--literal=$HOME"]
    );
    assert_eq!(
        saved.settings.daemon.agents[0]
            .as_custom()
            .expect("custom")
            .env["KEEP"],
        "original"
    );
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
    assert_eq!(
        saved.settings.daemon.agents[0]
            .as_custom()
            .expect("custom")
            .env["KEEP"],
        "original"
    );
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
        .save_agents(&mut document, vec![spec.clone().into()])
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
        page.update(cx, |page, cx| {
            page.edit(spec.clone(), Some(spec.id.clone()), window, cx)
        });
        window.render_frame(cx);
        assert!(
            window.try_find("agents-program").is_some(),
            "path stays editable while disconnected"
        );
        assert!(window.try_find("agents-grace").is_none());
        page.update(cx, |page, cx| {
            page.apply_check(&spec, Status::Installed("/test/adapter".into()));
            assert!(
                !page.draft_connected(cx),
                "finding a file is not a handshake"
            );
            page.apply_check(&spec, Status::Connected("/test/adapter".into()));
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
                candidate[0].as_custom().expect("custom").cancellation_grace,
                spec.cancellation_grace,
                "hidden advanced edits cannot bypass connection gating"
            );
            let check = page.checks.get_mut(&spec.id).expect("check");
            check.spec = candidate[0].as_custom().expect("custom").clone();
            check.status = Status::Checking;
            page.apply_check(&spec, Status::Connected("/test/adapter".into()));
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

#[gpui_kit::test]
fn managed_entries_have_no_advanced_editor_and_custom_edits_preserve_references(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("selected profile");
    let path = directory.path().join("selected-profile/settings.json");
    let store = Store::at(path.clone());
    let managed_spec = AgentSpec::acp("registry", Command::new("/managed/acp", []));
    let managed = managed_fixture(directory.path(), &managed_spec);
    let custom = AgentSpec::acp("custom", Command::new("/custom/acp", []));
    let mut document = store.load().expect("profile");
    store
        .save_agents(&mut document, vec![managed.clone(), custom.clone().into()])
        .expect("mixed registrations");
    cx.update(init);
    let mut view = None;
    let window = cx.open_window(size(px(960.), px(900.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(store.clone(), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |page, cx| {
            assert_eq!(page.catalog(), vec![custom.clone()]);
            page.edit(managed_spec.clone(), Some(custom.id.clone()), window, cx);
            assert!(
                matches!(page.editor, Editor::Idle),
                "a forged ID cannot open managed editing"
            );
            page.edit(managed_spec.clone(), None, window, cx);
            assert!(
                matches!(page.editor, Editor::Idle),
                "managed agents cannot be copied into custom drafts"
            );
        });
        window.render_frame(cx);
        assert!(window.try_find("agents-edit-0").is_none());
        assert!(window.try_find("agents-delete").is_none());
        click(window, "agents-edit-1", cx);
        fill(window, "agents-program", "/custom/edited", cx);
        click(window, "agents-save", cx);
        let saved = store.load().expect("saved selected profile");
        assert_eq!(saved.settings.daemon.agents[0], managed);
        assert_eq!(
            saved.settings.daemon.agents[1]
                .as_custom()
                .expect("custom")
                .command()
                .expect("command")
                .program,
            std::path::PathBuf::from("/custom/edited")
        );
        assert_eq!(view.read(cx).store.path(), path);
    })
    .expect("edit custom configuration while managed entries remain visible");
    let installation = cx.update(|cx| window::open(view.clone(), cx).expect("installation window"));
    cx.update_window(installation.into(), |_, window, cx| {
        click(window, "agents-add-custom", cx);
        fill(window, "agents-id", "registry", cx);
        fill(window, "agents-program", "/attempted/override", cx);
        let before = std::fs::read(&path).expect("before collision");
        click(window, "agents-save", cx);
        assert!(view.read(cx).dirty(cx));
        assert_eq!(std::fs::read(&path).expect("after collision"), before);
        click(window, "agents-cancel", cx);
    })
    .expect("reject custom collision with managed registration");
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "agents-edit-1", cx);
        click(window, "agents-delete", cx);
        click(window, "agents-save", cx);
        assert_eq!(
            store.load().expect("deleted custom").settings.daemon.agents,
            vec![managed.clone()]
        );

        view.update(cx, |page, cx| {
            // Simulate a delayed editor callback carrying the old resolved managed spec.
            page.editor = Editor::Edit {
                baseline: managed_spec.clone(),
                intent: EditIntent::Delete,
            };
            page.save(cx);
            assert!(page.candidate(cx).is_err());
        });
        assert_eq!(
            store
                .load()
                .expect("no stale deletion")
                .settings
                .daemon
                .agents,
            vec![managed.clone()]
        );
    })
    .expect("managed ownership and custom preservation");
}

#[gpui_kit::test]
fn stable_id_edits_follow_reordered_custom_and_managed_registrations(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("settings");
    let store = Store::at(directory.path().join("settings.json"));
    let target = AgentSpec::acp("target", Command::new("/custom/target", []));
    let other = AgentSpec::acp("other", Command::new("/custom/other", []));
    let managed = managed_fixture(
        directory.path(),
        &AgentSpec::acp("managed", Command::new("/managed/acp", [])),
    );
    let mut document = store.load().expect("document");
    store
        .save_agents(
            &mut document,
            vec![managed.clone(), target.clone().into(), other.clone().into()],
        )
        .expect("mixed catalog");
    cx.update(init);
    let mut view = None;
    let window = cx.open_window(size(px(960.), px(900.)), |window, cx| {
        let page = cx.new(|cx| Agents::new(store.clone(), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    let view = view.expect("page");
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |page, cx| {
            page.edit(target.clone(), Some(target.id.clone()), window, cx);
            assert!(!page.dirty(cx));
            // A catalog refresh may reload a clean editor without closing it.
            store
                .save_agents(
                    &mut document,
                    vec![other.clone().into(), managed.clone(), target.clone().into()],
                )
                .expect("reordered catalog");
            page.accept_document(store.load().expect("refreshed document"));
            assert_eq!(page.editor.selected(), Some(&target.id));
            page.form.program.update(cx, |input, cx| {
                input.set_value("/custom/changed", window, cx);
            });
            page.open_installation(cx);
            page.show_section(Some(1), cx);
            assert!(page.dirty(cx), "navigation retains the edited form");
        });
        window.render_frame(cx);
        let form = window.find("agents-program").bounds();
        let target_card = window.find("agents-card-2").bounds();
        assert!(form.top() >= target_card.top() && form.bottom() <= target_card.bottom());
        view.update(cx, |page, cx| {
            page.save(cx);
            assert!(matches!(page.editor, Editor::Idle));
            let saved = store.load().expect("saved");
            assert_eq!(saved.settings.daemon.agents[0], other.clone().into());
            assert_eq!(saved.settings.daemon.agents[1], managed);
            let changed = saved.settings.daemon.agents[2].as_custom().expect("target");
            assert_eq!(changed.id, target.id);
            assert_eq!(
                changed.command().expect("command").program,
                std::path::PathBuf::from("/custom/changed")
            );
            assert!(
                page.checks.contains_key(&target.id),
                "post-save check follows the saved ID"
            );

            // An open deletion editor also follows its ID through another reorder.
            page.edit(other.clone(), Some(other.id.clone()), window, cx);
            let mut refreshed = store.load().expect("current");
            let mut registrations = refreshed.settings.daemon.agents.clone();
            registrations.rotate_left(1);
            store
                .save_agents(&mut refreshed, registrations)
                .expect("reorder before deletion");
            page.accept_document(refreshed);
            page.editor.mark_delete();
            page.save(cx);
            assert_eq!(
                store.load().expect("deleted").settings.daemon.agents,
                vec![managed.clone(), changed.clone().into()]
            );
        });
    })
    .expect("stable-ID editing and deletion");
}

#[gpui_kit::test]
fn stale_custom_baseline_rejects_external_replacement_for_update_and_delete(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("settings");
    let store = Store::at(directory.path().join("settings.json"));
    let original = AgentSpec::acp("same-id", Command::new("/custom/original", []));
    let replacement = AgentSpec::acp("same-id", Command::new("/custom/replacement", []));
    let mut document = store.load().expect("document");
    store
        .save_agents(&mut document, vec![original.clone().into()])
        .expect("original");
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
            page.edit(original.clone(), Some(original.id.clone()), window, cx);
            page.form
                .program
                .update(cx, |input, cx| input.set_value("/custom/draft", window, cx));
            store
                .save_agents(&mut document, vec![replacement.clone().into()])
                .expect("external replacement");
            page.save(cx);
            assert!(page.dirty(cx), "Store conflict retains the draft");
            assert_eq!(page.form.values(cx).program, "/custom/draft");
            page.accept_document(store.load().expect("external refresh"));
            assert!(matches!(page.candidate(cx), Err(DraftError::Changed)));
            page.editor.mark_delete();
            assert!(matches!(page.candidate(cx), Err(DraftError::Changed)));
            page.save(cx);
            assert!(page.editor.deleting(), "failed deletion retains its intent");
            assert_eq!(
                store
                    .load()
                    .expect("not overwritten")
                    .settings
                    .daemon
                    .agents,
                vec![replacement.clone().into()]
            );
            page.editor = Editor::Idle;
            page.edit(original.clone(), Some(original.id.clone()), window, cx);
            assert!(
                matches!(page.editor, Editor::Idle),
                "stale callback cannot reopen the replaced spec"
            );
        });
    })
    .expect("stale custom replacement");
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
