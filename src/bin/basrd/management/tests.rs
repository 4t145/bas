use super::*;
mod fixture;
mod lifecycle;
mod persistence;

#[tokio::test]
async fn installation_registers_exact_boundaries_and_removal_keeps_credentials_and_versions() {
    let fixture = fixture::Fixture::serve(None, false).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let spec = fixture::install(&manager)
        .await
        .expect("managed installation");
    assert_eq!(
        spec.command().expect("ACP command").args,
        ["literal argument", "$NOT_EXPANDED"]
    );
    assert_eq!(spec.env["FIXTURE_ENV"], "literal $value");
    let executable = spec.command().expect("ACP command").program.clone();
    assert!(executable.is_file());
    assert!(executable.is_absolute());
    assert!(!executable.to_string_lossy().contains(".install-"));
    let managed = fixture::managed(&manager);
    assert_eq!(
        installation::load_record(&managed)
            .expect("installation record")
            .spec,
        spec
    );
    let settings = manager
        .store()
        .expect("settings store")
        .load()
        .expect("saved settings");
    assert!(
        settings
            .settings
            .daemon
            .agents
            .contains(&AgentRegistration::Managed(managed.clone()))
    );
    let credentials = directory.path().join("provider-credentials");
    fs::write(&credentials, "retain").expect("credential fixture");
    let agents = manager
        .remove("fixture-agent")
        .await
        .expect("remove managed registration");
    assert!(
        agents
            .iter()
            .all(|spec| spec.id().as_str() != "fixture-agent")
    );
    assert!(executable.is_file());
    assert!(credentials.is_file());
    assert!(installation::load_record(&managed).is_ok());
}

#[tokio::test]
async fn offline_refresh_uses_cached_registry_and_custom_missing_agents_remain_visible() {
    let fixture = fixture::Fixture::serve(None, false).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let missing = AgentSpec::acp(
        "custom-missing",
        Command::new(directory.path().join("absent"), []),
    );
    let initial = manager
        .catalog(
            vec![AgentSpec::embedded().into(), missing.clone().into()],
            false,
        )
        .await
        .expect("initial catalog");
    assert!(initial.warning.is_none());
    assert!(
        initial
            .entries
            .iter()
            .any(|entry| entry.id == "fixture-agent" && entry.status == Status::NotInstalled)
    );
    assert!(
        initial
            .entries
            .iter()
            .any(|entry| entry.id == "custom-missing"
                && entry.source == Source::Custom
                && entry.status == Status::Missing)
    );
    let url = fixture.url.clone();
    drop(fixture);
    let offline = manager
        .catalog(vec![missing.into()], true)
        .await
        .expect("cached offline catalog");
    assert!(offline.warning.expect("offline warning").contains("缓存"));
    assert!(
        manager
            .catalog(vec![AgentSpec::embedded().into()], false)
            .await
            .expect("subsequent cached catalog")
            .warning
            .is_some()
    );
    assert!(
        offline
            .entries
            .iter()
            .any(|entry| entry.id == "fixture-agent")
    );
    let restarted = fixture::manager(&directory, &url);
    assert!(
        restarted
            .catalog(vec![AgentSpec::embedded().into()], false)
            .await
            .expect("cache after restart")
            .entries
            .iter()
            .any(|entry| entry.id == "fixture-agent")
    );
}

#[tokio::test]
async fn bad_checksum_and_manual_identity_leave_settings_and_records_untouched() {
    let fixture = fixture::Fixture::serve(Some("0".repeat(64)), false).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    assert!(
        fixture::install(&manager)
            .await
            .expect_err("bad checksum")
            .to_string()
            .contains("checksum")
    );
    assert!(!manager.store().expect("settings store").path().exists());
    assert!(!manager.root.join("installations.json").exists());
    let store = manager.store().expect("settings store");
    let mut document = store.load().expect("default settings");
    let custom = AgentSpec::acp("fixture-agent", Command::new("custom-command", []));
    store
        .save_agents(&mut document, vec![custom.clone().into()])
        .expect("custom registration");
    assert!(
        fixture::install(&manager)
            .await
            .expect_err("custom identity conflict")
            .to_string()
            .contains("configured manually")
    );
    assert_eq!(
        store
            .load()
            .expect("preserved settings")
            .settings
            .daemon
            .agents,
        vec![AgentRegistration::Custom(custom)]
    );
}

#[tokio::test]
async fn settings_edits_during_download_are_preserved_without_registration() {
    let fixture = fixture::Fixture::serve(None, true).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let install_manager = manager.clone();
    let installation = tokio::spawn(async move { fixture::install(&install_manager).await });
    fixture.started.notified().await;
    let store = manager.store().expect("settings store");
    let mut document = store.load().expect("settings during download");
    let custom = AgentSpec::acp("external-edit", Command::new("external-command", []));
    store
        .save_agents(
            &mut document,
            vec![AgentSpec::embedded().into(), custom.clone().into()],
        )
        .expect("external settings edit");
    fixture.release.notify_one();
    assert!(
        installation
            .await
            .expect("installation task")
            .expect_err("settings conflict")
            .to_string()
            .contains("changed externally")
    );
    assert!(
        store
            .load()
            .expect("preserved settings")
            .settings
            .daemon
            .agents
            .contains(&AgentRegistration::Custom(custom))
    );
    let retained = fs::read_dir(manager.root.join("agents/fixture-agent"))
        .expect("versions")
        .next()
        .expect("retained version")
        .expect("version path")
        .path();
    assert!(retained.join(basr::config::INSTALLATION_FILE).is_file());
}

#[tokio::test]
async fn dropping_progress_stream_cancels_download_and_cleans_staging() {
    let fixture = fixture::Fixture::serve(None, true).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let (sender, receiver) = mpsc::channel(32);
    let install_manager = manager.clone();
    let installation =
        tokio::spawn(async move { install_manager.install("fixture-agent", sender).await });
    fixture.started.notified().await;
    drop(receiver);
    assert!(
        installation
            .await
            .expect("installation task")
            .expect_err("cancelled install")
            .to_string()
            .contains("cancelled")
    );
    assert!(!manager.root.join("installations.json").exists());
    assert!(!manager.store().expect("settings store").path().exists());
    assert!(
        fs::read_dir(manager.root.join("agents/fixture-agent"))
            .expect("version directory")
            .next()
            .is_none()
    );
}

#[test]
fn managed_npm_readiness_checks_the_js_entrypoint_and_pinned_dependency_versions() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let executable = directory.path().join("node");
    fs::write(&executable, b"fixture runtime").expect("runtime fixture");
    artifact::executable(&executable).expect("executable runtime fixture");
    let installed = directory.path().join("agent");
    fs::create_dir_all(installed.join("node_modules/fixture/dist")).expect("npm tree fixture");
    fs::write(
        installed.join("node_modules/fixture/package.json"),
        br#"{"version":"1.0.0"}"#,
    )
    .expect("package metadata");
    let entrypoint = installed.join("node_modules/fixture/dist/index.js");
    let spec = AgentSpec::acp(
        "fixture",
        Command::new(executable, [entrypoint.to_string_lossy().into_owned()]),
    );
    let manifest = serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","version":"1.0.0","distribution":{"npx":{"package":"fixture@1.0.0"}}})).expect("npm manifest");
    let record = Record {
        manifest,
        directory: installed.clone(),
        spec,
        runtime: None,
        dependencies: BTreeMap::from([("fixture".into(), "1.0.0".into())]),
        launch: None,
    };
    assert!(
        !record_available(&record),
        "an installed Node runtime does not imply that the backend JS exists"
    );
    fs::write(entrypoint, "fixture").expect("JS entrypoint");
    assert!(record_available(&record));
    fs::write(
        installed.join("node_modules/fixture/package.json"),
        br#"{"version":"2.0.0"}"#,
    )
    .expect("changed dependency");
    assert!(!record_available(&record));
}

#[cfg(unix)]
#[test]
fn configured_path_override_is_used_for_custom_backend_readiness() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let executable = directory.path().join("basr-custom-path-fixture");
    fs::write(&executable, b"fixture executable").expect("custom executable");
    artifact::executable(&executable).expect("executable permission");
    let mut spec = AgentSpec::acp("custom-path", Command::new("basr-custom-path-fixture", []));
    assert!(!available(&spec));
    spec.env.insert(
        "PATH".into(),
        directory.path().to_string_lossy().into_owned(),
    );
    assert!(available(&spec));
}
