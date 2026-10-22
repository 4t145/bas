use super::*;

fn fixture(scope: Scope) -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().expect("scoped configuration");
    let name = if matches!(scope, Scope::Daemon) {
        "daemon.json"
    } else {
        "settings.json"
    };
    let store = Store {
        path: directory.path().join(name),
        import_legacy: true,
        scope,
    };
    (directory, store)
}

#[test]
fn frontend_ignores_and_preserves_opaque_legacy_daemon() {
    let directory = tempfile::tempdir().expect("configuration");
    let path = directory.path().join("settings.json");
    let backend = serde_json::json!({"agents": [], "acp": {"future_policy": "unknown"}});
    fs::write(&path, serde_json::json!({"daemon": backend}).to_string()).expect("legacy");
    let store = FrontendStore::at(path.clone());
    let mut document = store.load().expect("backend cannot block frontend startup");
    let appearance = Appearance {
        ui_font_size: Some(17.0),
        ..Default::default()
    };
    store
        .save_appearance(&mut document, appearance.clone())
        .expect("frontend save");
    let saved: serde_json::Value = parse(&path, &fs::read(&path).expect("saved")).expect("JSON");
    assert_eq!(
        saved["daemon"], backend,
        "legacy backend must not be erased"
    );
    assert_eq!(document.settings.appearance, appearance);
    assert!(
        DaemonStore::at(path).load().is_err(),
        "backend still validates its own fields"
    );
}

#[test]
fn daemon_ignores_opaque_legacy_frontend_and_revalidates_policy_on_save() {
    let directory = tempfile::tempdir().expect("configuration");
    let path = directory.path().join("legacy.json");
    let frontend = serde_json::json!({"future_connection": "unknown"});
    fs::write(
        &path,
        serde_json::json!({
            "appearance": {"ui_font_size": 0}, "frontend": frontend,
            "daemon": {
                "agents": [{"id": "a", "kind": "acp", "command": {"program": "unused"}}],
                "acp": {"default_agent": "a", "pool_size": 2}
            }
        })
        .to_string(),
    )
    .expect("legacy");
    let store = DaemonStore::at(path.clone());
    let mut document = store.load().expect("frontend cannot block daemon");
    let policy = document.settings.acp.clone();
    let mut agents = document.settings.agents.clone();
    agents.push(AgentSpec::embedded().into());
    store
        .save_agents(&mut document, agents)
        .expect("backend save");
    assert_eq!(document.settings.acp, policy);
    let saved = fs::read(&path).expect("saved");
    let json: serde_json::Value = parse(&path, &saved).expect("JSON");
    assert_eq!(json["frontend"], frontend);
    assert_eq!(json["appearance"]["ui_font_size"], 0);
    assert!(
        store
            .save_agents(&mut document, vec![AgentSpec::embedded().into()])
            .is_err()
    );
    assert_eq!(fs::read(&path).expect("preserved"), saved);
}

#[test]
fn default_legacy_import_preserves_policy_even_when_frontend_saves_first() {
    let (directory, store) = fixture(Scope::Daemon);
    let legacy = directory.path().join("settings.json");
    fs::write(
        &legacy,
        serde_json::json!({
            "daemon": {"agents": [{"id": "embedded", "kind": "embedded"}],
                       "acp": {"idle_timeout_secs": 77, "pool_size": 0}}
        })
        .to_string(),
    )
    .expect("legacy unified");
    fs::write(directory.path().join("agents.json"), "invalid older source").expect("older");
    let frontend = FrontendStore::at(legacy.clone());
    let mut preferences = frontend.load().expect("preferences");
    frontend
        .save_frontend(&mut preferences, Frontend::default())
        .expect("frontend first");
    let before = fs::read(&legacy).expect("legacy preserved");
    let mut document = store.load().expect("import");
    assert!(!store.path.exists(), "reading does not migrate");
    assert_eq!(
        document
            .settings
            .daemon
            .acp
            .idle_timeout_secs
            .expect("timeout")
            .get(),
        77
    );
    let agents = document.settings.daemon.agents.clone();
    store
        .save_agents(&mut document, agents)
        .expect("materialize daemon config");
    assert_eq!(fs::read(&legacy).expect("legacy unchanged"), before);
    let json: serde_json::Value =
        parse(&store.path, &fs::read(&store.path).expect("new")).expect("JSON");
    assert!(json.get("daemon").is_none() && json.get("appearance").is_none());
    assert_eq!(json["acp"]["pool_size"], 0);
    fs::write(&legacy, "now invalid").expect("bad legacy");
    assert!(store.load().is_ok(), "canonical config is independent");
    fs::write(&store.path, "invalid canonical").expect("bad canonical");
    assert!(
        store.load().is_err(),
        "never fall back from malformed canonical config"
    );
}

#[test]
fn new_files_and_default_paths_have_distinct_owners() {
    let frontend_default = FrontendStore::new(None);
    let daemon_default = DaemonStore::new(None);
    assert_eq!(
        frontend_default.path().file_name().expect("name"),
        "settings.json"
    );
    assert_eq!(
        daemon_default.path(),
        frontend_default.path().with_file_name("daemon.json")
    );
    let directory = tempfile::tempdir().expect("configuration");
    let frontend = FrontendStore::at(directory.path().join("custom.json"));
    let mut document = frontend.load().expect("default frontend");
    frontend
        .save_frontend(&mut document, Frontend::default())
        .expect("save");
    let json: serde_json::Value =
        parse(frontend.path(), &fs::read(frontend.path()).expect("file")).expect("JSON");
    assert!(json.get("daemon").is_none());
    assert_eq!(DaemonStore::new(None).path(), daemon_default.path());
}

#[test]
fn frontend_does_not_import_broken_agents_and_explicit_paths_do_not_import_neighbors() {
    let (directory, frontend) = fixture(Scope::Frontend);
    fs::write(directory.path().join("agents.json"), "invalid").expect("broken agents");
    assert!(frontend.load().is_ok());
    let daemon = DaemonStore::at(directory.path().join("custom-daemon.json"));
    assert_eq!(
        daemon.load().expect("explicit default").settings,
        crate::config::Daemon::default()
    );
    fs::write(
        directory.path().join("appearance.json"),
        r#"{"ui_font_size":0}"#,
    )
    .expect("bad appearance");
    assert!(
        frontend.load().is_err(),
        "own invalid legacy values still fail"
    );
}

#[test]
fn typed_stores_keep_section_conflicts_and_policy_edits() {
    let directory = tempfile::tempdir().expect("configuration");
    let path = directory.path().join("daemon.json");
    let store = DaemonStore::at(path.clone());
    let mut first = store.load().expect("first");
    let mut stale = first.clone();
    store
        .save_agents(
            &mut first,
            vec![AgentSpec::acp("a", crate::config::agent::Command::new("unused", [])).into()],
        )
        .expect("first writer");
    assert!(matches!(
        store.save_agents(&mut stale, vec![AgentSpec::embedded().into()]),
        Err(Error::Conflict { .. })
    ));
    let mut raw: serde_json::Value = parse(&path, &fs::read(&path).expect("read")).expect("JSON");
    raw["acp"]["pool_size"] = 3.into();
    fs::write(&path, raw.to_string()).expect("concurrent policy");
    let agents = first.settings.agents.clone();
    store
        .save_agents(&mut first, agents)
        .expect("merge unrelated policy");
    assert_eq!(first.settings.acp.pool_size, 3);
}
