use super::*;
mod fixture;
mod lifecycle;
mod persistence;

#[tokio::test]
async fn catalog_resolves_configuration_paths_in_the_daemon() {
    let fixture = fixture::Fixture::serve(None, false).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let mut manager = fixture::manager(&directory, &fixture.url);
    let relative = PathBuf::from("relative-backend-settings.json");
    Arc::get_mut(&mut manager)
        .expect("the manager has no other owners")
        .store = Some(Store::at(relative.clone()));
    let catalog = manager
        .catalog(vec![AgentSpec::embedded().into()], false)
        .await
        .expect("backend catalog");
    assert_eq!(
        catalog.settings_path,
        Some(std::path::absolute(relative).expect("daemon working directory"))
    );
}

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
    assert!(retained.join(bas::config::INSTALLATION_FILE).is_file());
}
