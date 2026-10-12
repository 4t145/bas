use super::*;
use basr::config::{INSTALLATION_FILE, InstallationRef, ManagedAgent};

fn prepare(manager: &Manager, version: &str) -> ManagedAgent {
    let versions = manager.root.join("agents/fixture-agent");
    artifact::safe_directory(&versions).expect("versions directory");
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(&versions)
        .expect("staging directory");
    let destination = versions.join(uuid::Uuid::new_v4().to_string());
    fs::write(staging.path().join("agent"), "#!/bin/sh\nexit 0\n").expect("executable");
    artifact::executable(&staging.path().join("agent")).expect("executable permission");
    let manifest = serde_json::from_value(serde_json::json!({
        "id": "fixture-agent", "name": format!("Fixture {version}"), "version": version,
        "description": format!("Installed {version}"),
        "distribution": {"binary": {registry::platform(): {
            "archive": "http://127.0.0.1/agent", "cmd": "agent"
        }}}
    }))
    .expect("manifest");
    installation::finalize(
        staging,
        Record {
            manifest,
            spec: AgentSpec::acp("fixture-agent", Command::new(destination.join("agent"), [])),
            directory: destination,
            runtime: None,
            dependencies: BTreeMap::new(),
            launch: None,
        },
    )
    .expect("complete version")
}

fn activate(manager: &Manager, managed: ManagedAgent) {
    let store = manager.store().expect("settings store");
    let mut document = store.load().expect("settings snapshot");
    installation::activate(store, &mut document, managed).expect("activate version");
}

#[cfg(unix)]
#[test]
fn crash_fixture() {
    let Some(directory) = std::env::var_os("BASR_TEST_INSTALL_CRASH_DIRECTORY") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let phase = std::env::var("BASR_TEST_INSTALL_CRASH_PHASE").expect("crash phase");
    let manager = fixture::manager_at(&directory, "http://127.0.0.1:1");
    let managed = prepare(&manager, "2.0.0");
    fs::write(
        directory.join("prepared.json"),
        serde_json::to_vec(&managed).expect("reference"),
    )
    .expect("publish test reference");
    match phase.as_str() {
        "prepared" => {}
        "activated" => activate(&manager, managed),
        _ => panic!("unexpected crash phase"),
    }
    nix::sys::signal::kill(nix::unistd::Pid::this(), nix::sys::signal::Signal::SIGKILL)
        .expect("kill test process without running destructors");
    unreachable!("SIGKILL terminates the fixture");
}

#[cfg(unix)]
#[tokio::test]
async fn process_death_on_each_side_of_activation_keeps_a_resolvable_profile() {
    use std::os::unix::process::ExitStatusExt;
    for phase in ["prepared", "activated"] {
        let directory = tempfile::tempdir().expect("crash fixture directory");
        let manager = fixture::manager(&directory, "http://127.0.0.1:1");
        let old = prepare(&manager, "1.0.0");
        activate(&manager, old.clone());
        let mut command =
            tokio::process::Command::new(std::env::current_exe().expect("test executable"));
        command
            .args(["--exact", "management::tests::persistence::crash_fixture"])
            .env("BASR_TEST_INSTALL_CRASH_DIRECTORY", directory.path())
            .env("BASR_TEST_INSTALL_CRASH_PHASE", phase)
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(10), command.output())
            .await
            .expect("crash fixture deadline")
            .expect("crash fixture starts");
        assert_eq!(
            output.status.signal(),
            Some(nix::sys::signal::Signal::SIGKILL as i32),
            "fixture must reach crash point: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let prepared: ManagedAgent = serde_json::from_slice(
            &fs::read(directory.path().join("prepared.json")).expect("prepared reference"),
        )
        .expect("reference JSON");
        let restarted = fixture::manager(&directory, "http://127.0.0.1:1");
        let selected = fixture::managed(&restarted);
        assert_eq!(
            selected,
            if phase == "activated" {
                prepared.clone()
            } else {
                old.clone()
            }
        );
        assert!(selected.resolve().is_ok());
        assert!(installation::load_record(&old).is_ok());
        assert!(installation::load_record(&prepared).is_ok());
    }
}

#[tokio::test]
async fn profiles_share_artifacts_but_not_activation_upgrade_or_removal() {
    let fixture = fixture::Fixture::serve(None, false).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let first = fixture::manager(&directory, &fixture.url);
    let mut second = fixture::manager(&directory, &fixture.url);
    Arc::get_mut(&mut second).expect("isolated manager").store =
        Some(Store::at(directory.path().join("second.json")));
    let old = prepare(&first, "1.0.0");
    activate(&first, old.clone());
    activate(&second, old.clone());
    let second_bytes =
        fs::read(second.store().expect("second store").path()).expect("second settings");
    let upgraded = prepare(&first, "2.0.0");
    activate(&first, upgraded.clone());
    assert_eq!(fixture::managed(&first), upgraded);
    assert_eq!(fixture::managed(&second), old);
    assert_eq!(
        fs::read(second.store().expect("second store").path()).expect("second settings"),
        second_bytes
    );
    first.remove("fixture-agent").await.expect("remove first");
    assert_eq!(fixture::managed(&second), old);
    assert!(installation::load_record(&old).is_ok());
    assert!(installation::load_record(&upgraded).is_ok());
    second.remove("fixture-agent").await.expect("remove second");
    for manager in [&first, &second] {
        let agents = manager
            .store()
            .expect("store")
            .load()
            .expect("settings")
            .settings
            .daemon
            .agents;
        assert_eq!(agents, vec![AgentSpec::embedded().into()]);
    }
    assert!(!first.root.join("installations.json").exists());
}

#[test]
fn preparation_and_failed_activation_retain_complete_versions_and_previous_reference() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, "http://127.0.0.1:1");
    let old = prepare(&manager, "1.0.0");
    assert!(
        !manager.store().expect("store").path().exists(),
        "preparation does not activate"
    );
    activate(&manager, old.clone());
    let store = manager.store().expect("store");
    let mut stale = store.load().expect("snapshot before preparation");
    let new = prepare(&manager, "2.0.0");
    assert_eq!(
        fixture::managed(&manager),
        old,
        "interruption before activation leaves the old reference"
    );
    let mut external = store.load().expect("external snapshot");
    let mut registrations = external.settings.daemon.agents.clone();
    registrations.push(AgentSpec::acp("external", Command::new("external", [])).into());
    store
        .save_agents(&mut external, registrations)
        .expect("external edit");
    let bytes = fs::read(store.path()).expect("settings before conflict");
    assert!(installation::activate(store, &mut stale, new.clone()).is_err());
    assert_eq!(fs::read(store.path()).expect("unchanged settings"), bytes);
    assert_eq!(fixture::managed(&manager), old);
    assert!(
        installation::load_record(&new).is_ok(),
        "complete unreferenced artifact survives failed save"
    );
    assert!(installation::load_record(&old).is_ok());
}

#[tokio::test]
async fn identical_custom_command_never_acquires_managed_ownership() {
    let fixture = fixture::Fixture::serve(None, false).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let managed = prepare(&manager, "1.0.0");
    let spec = managed.resolve().expect("launch projection");
    let store = manager.store().expect("store");
    let mut document = store.load().expect("settings");
    store
        .save_agents(&mut document, vec![spec.clone().into()])
        .expect("custom config");
    let bytes = fs::read(store.path()).expect("custom settings");
    assert!(
        fixture::install(&manager)
            .await
            .expect_err("custom collision")
            .to_string()
            .contains("configured manually")
    );
    assert!(manager.remove("fixture-agent").await.is_err());
    assert!(installation::activate(store, &mut document, managed).is_err());
    assert_eq!(fs::read(store.path()).expect("unchanged settings"), bytes);
    let catalog = manager
        .catalog(vec![spec.into()], false)
        .await
        .expect("catalog");
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == "fixture-agent")
        .expect("entry");
    assert_eq!(entry.source, Source::Custom);
    assert_eq!(entry.installed_version, None);
}

#[tokio::test]
async fn catalog_uses_selected_descriptor_metadata_and_separate_latest_version() {
    let fixture = fixture::Fixture::serve(None, false).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let managed = prepare(&manager, "0.5.0");
    let registrations = vec![AgentRegistration::Managed(managed.clone())];
    let catalog = manager
        .catalog(registrations.clone(), false)
        .await
        .expect("catalog");
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == "fixture-agent")
        .expect("entry");
    assert_eq!(entry.name, "Fixture 0.5.0");
    assert_eq!(entry.description, "Installed 0.5.0");
    assert_eq!(entry.version, "1.0.0");
    assert_eq!(entry.installed_version.as_deref(), Some("0.5.0"));
    assert_eq!(entry.source, Source::Registry);
    drop(fixture);
    manager.registry.lock().await.registry = None;
    fs::remove_file(manager.root.join("registry.json")).expect("remove registry cache");
    let offline = manager
        .catalog(registrations, true)
        .await
        .expect("descriptor without registry");
    assert!(offline.warning.is_some());
    let entry = offline
        .entries
        .iter()
        .find(|entry| entry.id == "fixture-agent")
        .expect("offline entry");
    assert_eq!(entry.name, "Fixture 0.5.0");
    assert_eq!(entry.installed_version.as_deref(), Some("0.5.0"));
    assert_eq!(entry.status, Status::Installed);
}

#[test]
fn digest_missing_descriptor_and_rebound_identity_fail_closed() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, "http://127.0.0.1:1");
    let managed = prepare(&manager, "1.0.0");
    let path = &managed.installation.path;
    let bytes = fs::read(path).expect("descriptor");
    fs::write(path, b"{}").expect("tamper");
    assert!(installation::load_record(&managed).is_err());
    fs::remove_file(path).expect("remove descriptor");
    assert!(installation::load_record(&managed).is_err());
    for (field, value) in [
        ("schema_version", serde_json::json!(2)),
        ("directory", serde_json::json!(directory.path())),
        (
            "directory",
            serde_json::json!(path.parent().expect("parent").join("..")),
        ),
        (
            "manifest",
            serde_json::json!({"id": "other", "name": "Other", "version": "1.0.0", "distribution": {}}),
        ),
    ] {
        let mut descriptor: serde_json::Value =
            serde_json::from_slice(&bytes).expect("descriptor JSON");
        descriptor[field] = value;
        let changed = serde_json::to_vec(&descriptor).expect("changed descriptor");
        fs::write(path, &changed).expect("changed descriptor file");
        let rebound = ManagedAgent {
            id: managed.id.clone(),
            installation: InstallationRef::from_bytes(path.clone(), &changed).expect("new digest"),
        };
        assert!(
            installation::load_record(&rebound).is_err(),
            "{field} must be validated independently of digest"
        );
    }
    assert_eq!(path.file_name().expect("filename"), INSTALLATION_FILE);
}

#[cfg(unix)]
#[test]
fn descriptor_and_directory_symlinks_fail_closed() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, "http://127.0.0.1:1");
    let managed = prepare(&manager, "1.0.0");
    let path = &managed.installation.path;
    let moved = directory.path().join("descriptor.json");
    fs::rename(path, &moved).expect("move descriptor");
    std::os::unix::fs::symlink(&moved, path).expect("descriptor symlink");
    assert!(installation::load_record(&managed).is_err());
    fs::remove_file(path).expect("remove symlink");
    fs::rename(&moved, path).expect("restore descriptor");
    let version = path.parent().expect("version directory");
    let moved = directory.path().join("moved-version");
    fs::rename(version, &moved).expect("move version");
    std::os::unix::fs::symlink(&moved, version).expect("version symlink");
    assert!(installation::load_record(&managed).is_err());
}
