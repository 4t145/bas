use super::super::*;
use bas::config::{Store, agent::Command};
use gpui_kit::{AppContext, TestAppContext, component::Root, size};

#[gpui_kit::test]
fn local_backend_profile_is_the_only_write_target_and_refresh_preserves_drafts(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().expect("profiles");
    let frontend_path = directory.path().join("frontend.json");
    let backend_path = directory.path().join("backend.json");
    std::fs::write(
        &frontend_path,
        r#"{"appearance":{"theme_source":"builtin"}}"#,
    )
    .expect("frontend");
    let frontend_bytes = std::fs::read(&frontend_path).expect("frontend bytes");
    let backend = Store::at(backend_path.clone());
    let original = bas::config::AgentSpec::acp("backend-only", Command::new("/backend/agent", []));
    let mut document = backend.load().expect("backend");
    backend
        .save_agents(&mut document, vec![original.clone().into()])
        .expect("backend fixture");
    cx.update(super::super::super::tests::init);
    let mut view = None;
    let window = cx.open_window(size(px(900.), px(720.)), |window, cx| {
        let agents = cx.new(|cx| Agents::new(Store::at(frontend_path.clone()), window, cx));
        view = Some(agents.clone());
        Root::new(agents, window, cx)
    });
    let agents = view.expect("agents");
    cx.update_window(window.into(), |_, window, cx| {
        agents.update(cx, |agents, cx| {
            let catalog = super::catalog(backend_path.clone());
            agents.apply_catalog(catalog.clone(), true, cx);
            assert_eq!(agents.settings_path(), Some(backend_path.as_path()));
            assert_eq!(agents.catalog(), vec![original.clone()]);
            agents.edit(original.clone(), Some(original.id.clone()), window, cx);
            agents.form.program.update(cx, |input, cx| {
                input.set_value("/backend/edited", window, cx)
            });
            assert!(agents.dirty(cx));
            agents.apply_catalog(catalog, true, cx);
            assert_eq!(agents.form.values(cx).program, "/backend/edited");
            assert!(agents.dirty(cx));
            agents.save(cx);
            assert!(!agents.dirty(cx));
        });
    })
    .expect("edit verified backend");
    assert_eq!(
        std::fs::read(&frontend_path).expect("frontend unchanged"),
        frontend_bytes
    );
    assert_eq!(
        backend
            .load()
            .expect("saved backend")
            .settings
            .daemon
            .agents[0]
            .as_custom()
            .expect("custom")
            .command()
            .expect("command")
            .program,
        std::path::PathBuf::from("/backend/edited"),
    );
}

#[gpui_kit::test]
fn unverified_profiles_and_backend_switches_cannot_redirect_a_dirty_draft(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("profiles");
    let initial_path = directory.path().join("first.json");
    let other_path = directory.path().join("second.json");
    let store = Store::at(initial_path.clone());
    let original = bas::config::AgentSpec::acp("custom", Command::new("/original", []));
    let mut document = store.load().expect("initial");
    store
        .save_agents(&mut document, vec![original.clone().into()])
        .expect("initial fixture");
    std::fs::write(&other_path, "{}").expect("other profile");
    let original_bytes = std::fs::read(&initial_path).expect("original bytes");
    let other_bytes = std::fs::read(&other_path).expect("other bytes");
    cx.update(super::super::super::tests::init);
    let mut view = None;
    let window = cx.open_window(size(px(900.), px(720.)), |window, cx| {
        let agents = cx.new(|cx| Agents::new(store.clone(), window, cx));
        view = Some(agents.clone());
        Root::new(agents, window, cx)
    });
    let agents = view.expect("agents");
    cx.update_window(window.into(), |_, window, cx| {
        agents.update(cx, |agents, cx| {
            agents.apply_catalog(super::catalog(initial_path.clone()), true, cx);
            agents.edit(original.clone(), Some(original.id.clone()), window, cx);
            agents
                .form
                .program
                .update(cx, |input, cx| input.set_value("/draft", window, cx));
            agents.apply_catalog(super::catalog(other_path.clone()), true, cx);
            assert!(agents.settings_path().is_none());
            assert_eq!(agents.store.path(), initial_path);
            assert!(agents.dirty(cx));
            agents.save(cx);
            assert!(agents.dirty(cx));
            assert_eq!(agents.form.values(cx).program, "/draft");
            agents.editor = super::super::super::Editor::Idle;
            for (path, local, can_install) in [
                (Some(other_path.clone()), false, true),
                (Some(directory.path().join("absent.json")), true, true),
                (Some(std::path::PathBuf::from("relative.json")), true, true),
                (None, true, true),
                (Some(other_path.clone()), true, false),
            ] {
                let mut catalog = super::catalog(other_path.clone());
                catalog.settings_path = path;
                catalog.can_install = can_install;
                agents.apply_catalog(catalog, local, cx);
                assert!(!agents.registry.editable());
                assert!(agents.settings_path().is_none());
                assert_eq!(agents.store.path(), initial_path);
            }
            agents.apply_catalog(super::catalog(other_path.clone()), true, cx);
            assert_eq!(agents.settings_path(), Some(other_path.as_path()));
        });
    })
    .expect("profile access safeguards");
    assert_eq!(
        std::fs::read(initial_path).expect("original untouched"),
        original_bytes
    );
    assert_eq!(
        std::fs::read(other_path).expect("other untouched"),
        other_bytes
    );
}
