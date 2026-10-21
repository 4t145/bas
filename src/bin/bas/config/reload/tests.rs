use super::*;
use crate::{
    agent::{Catalog, Command, LocalHost},
    session::{AttachOptions, CreateRequest, Limits},
};
use bas::config::{AgentRegistration, AgentSpec, InstallationRef, ManagedAgent};
use bas::model::{EventKind, Owner};

#[tokio::test]
async fn policy_only_reload_and_invalid_combined_update_are_atomic() {
    let directory = tempfile::tempdir().expect("settings directory");
    let path = directory.path().join("settings.json");
    let mut settings = bas::config::Settings::default();
    settings.daemon.agents = vec![AgentSpec::acp("fixture", Command::new("unused", [])).into()];
    std::fs::write(&path, serde_json::to_vec(&settings).expect("JSON")).expect("settings");
    let mut source = CatalogSource::settings(Some(path.clone())).expect("source");
    let initial = source.load_configuration().expect("initial settings");
    let registry = Arc::new(Registry::new(
        Arc::new(Catalog::from_registrations(initial.agents.clone()).expect("catalog")),
        Arc::new(LocalHost::default()),
        Limits::default(),
    ));
    let before = registry.catalog();
    settings.daemon.acp.idle_timeout_secs = std::num::NonZeroU64::new(900);
    std::fs::write(&path, serde_json::to_vec(&settings).expect("JSON")).expect("update");
    let stopping = CancellationToken::new();
    let watcher = watch_catalog(source, initial.agents, registry.clone(), stopping.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while registry.acp_policy() != settings.daemon.acp {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("policy-only update");
    assert!(
        Arc::ptr_eq(&before, &registry.catalog()),
        "unchanged catalog generation"
    );
    let valid_policy = registry.acp_policy();
    settings.daemon.agents = vec![AgentSpec::embedded().into()];
    settings.daemon.acp.pool_size = bas::config::MAX_ACP_POOL_RESERVE + 1;
    std::fs::write(&path, serde_json::to_vec(&settings).expect("JSON")).expect("invalid update");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(Arc::ptr_eq(&before, &registry.catalog()));
    assert_eq!(registry.acp_policy(), valid_policy);
    stopping.cancel();
    watcher.await.expect("watcher stopped");
    registry.shutdown().await;
}

fn write_settings(path: &std::path::Path, agents: Vec<AgentSpec>) {
    write_registrations(
        path,
        agents.into_iter().map(AgentRegistration::from).collect(),
    );
}

fn write_registrations(path: &std::path::Path, agents: Vec<AgentRegistration>) {
    let mut settings = bas::config::Settings::default();
    settings.daemon.agents = agents;
    std::fs::write(path, serde_json::to_vec(&settings).unwrap()).unwrap();
}

fn request(id: &str) -> CreateRequest {
    CreateRequest {
        agent: id.into(),
        owner: Owner::current(),
        cwd: None,
        title: None,
    }
}

fn managed(directory: &std::path::Path, spec: &AgentSpec) -> AgentRegistration {
    let path = directory.join(bas::config::INSTALLATION_FILE);
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "spec": spec,
        "directory": directory,
        "manifest": {
            "id": spec.id, "name": "Fixture", "version": "1.0.0",
            "distribution": {}
        },
        "runtime": null
    }))
    .expect("launch descriptor");
    let installation = InstallationRef::from_bytes(path.clone(), &bytes).expect("reference");
    std::fs::write(path, bytes).expect("descriptor");
    AgentRegistration::Managed(ManagedAgent {
        id: spec.id.clone(),
        installation,
    })
}

#[test]
fn a_launch_projection_alone_is_not_a_complete_daemon_descriptor() {
    let directory = tempfile::tempdir().expect("installation directory");
    let spec = AgentSpec::acp("fixture", Command::new("fixture", []));
    let path = directory.path().join(bas::config::INSTALLATION_FILE);
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1, "spec": spec
    }))
    .expect("launch projection");
    std::fs::write(&path, &bytes).expect("projection file");
    let reference = ManagedAgent {
        id: spec.id,
        installation: InstallationRef::from_bytes(path, &bytes).expect("reference"),
    };
    assert!(
        reference.resolve().is_ok(),
        "shared launch projection is readable"
    );
    assert!(Catalog::from_registrations(vec![AgentRegistration::Managed(reference)]).is_err());
}

#[test]
fn identical_launch_spec_with_different_provenance_publishes_a_new_generation() {
    let directory = tempfile::tempdir().expect("installation directory");
    let spec = AgentSpec::acp("fixture", Command::new("fixture", Vec::new()));
    let registration = managed(directory.path(), &spec);
    let registry = Registry::new(
        Arc::new(Catalog::from_registrations(vec![registration.clone()]).expect("managed catalog")),
        Arc::new(LocalHost::default()),
        Limits::default(),
    );
    let before = registry.catalog();
    assert!(before.managed(&spec.id).is_some());
    registry
        .reload_catalog(|| Ok(vec![registration]))
        .expect("unchanged reload");
    assert!(Arc::ptr_eq(&before, &registry.catalog()));
    registry
        .reload_catalog(|| Ok(vec![spec.clone().into()]))
        .expect("custom reload");
    let after = registry.catalog();
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(before.spec(&spec.id), after.spec(&spec.id));
    assert!(after.managed(&spec.id).is_none());
}

async fn wait_catalog(registry: &Registry, expected: &[AgentSpec]) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let actual: Vec<_> = registry
                .catalog()
                .specs()
                .map(|spec| (**spec).clone())
                .collect();
            if actual == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("catalog reload");
}

#[cfg(unix)]
#[tokio::test]
async fn reloading_specs_preserves_existing_acp_sessions_and_shutdown_is_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut spec = AgentSpec::acp(
        "fixture",
        Command::new(
            "sh",
            [
                "-c".to_owned(),
                include_str!("../../agent/acp/assets/fixture.sh").to_owned(),
            ],
        ),
    );
    spec.env.insert("BAS_FIXTURE_MODE".into(), "normal".into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
        directory
            .path()
            .join("old.pid")
            .to_string_lossy()
            .into_owned(),
    );
    let registration = managed(directory.path(), &spec);
    write_registrations(&path, vec![registration.clone()]);
    let mut source = CatalogSource::settings(Some(path.clone())).unwrap();
    let agents = source.load().unwrap();
    let registry = Arc::new(Registry::new(
        Arc::new(Catalog::from_registrations(agents.clone()).unwrap()),
        Arc::new(LocalHost::default()),
        Limits::default(),
    ));
    let old = registry.create(request("fixture")).await.unwrap();
    let valid_catalog = registry.catalog();
    let descriptor = &registration
        .managed()
        .expect("managed fixture")
        .installation
        .path;
    let original = std::fs::read(descriptor).expect("descriptor bytes");
    let mut tampered = original.clone();
    tampered.push(b'\n');
    std::fs::write(descriptor, tampered).expect("tamper descriptor");
    assert!(registry.reload_catalog(|| source.load()).is_err());
    assert!(Arc::ptr_eq(&valid_catalog, &registry.catalog()));
    assert!(valid_catalog.validate(&spec).is_err());
    assert!(registry.create(request("fixture")).await.is_err());
    std::fs::remove_file(descriptor).expect("remove descriptor");
    assert!(registry.reload_catalog(|| source.load()).is_err());
    assert!(Arc::ptr_eq(&valid_catalog, &registry.catalog()));
    assert!(valid_catalog.validate(&spec).is_err());
    let (updates, _inbox) = tokio::sync::mpsc::channel(8);
    let restore = crate::agent::LaunchRequest {
        session: bas::model::SessionId::new(),
        spec: Arc::new(spec.clone()),
        cwd: directory.path().to_owned(),
        additional_directories: Vec::new(),
        host: Arc::new(LocalHost::default()),
        updates,
    };
    assert!(
        valid_catalog
            .restore(restore, "saved".into())
            .await
            .is_err()
    );
    use bas::service::pb::{self, bas_server::Bas};
    let service = crate::service::Service::new(registry.clone());
    let error = service
        .check_agent(tonic::Request::new(pb::CheckAgentRequest {
            agent: spec.id.to_string(),
            expected_spec_json: serde_json::to_vec(&spec).expect("expected spec"),
            connect: true,
        }))
        .await
        .expect_err("probe must not spawn from an invalid reference");
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    std::fs::write(descriptor, original).expect("restore descriptor");
    let stopping = CancellationToken::new();
    let watcher = watch_catalog(source, agents, Arc::clone(&registry), stopping.clone());

    // Same id, new environment: the actual new process must receive the new spec.
    spec.env
        .insert("BAS_FIXTURE_MODE".into(), "early_update".into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
        directory
            .path()
            .join("new.pid")
            .to_string_lossy()
            .into_owned(),
    );
    write_settings(&path, vec![spec.clone()]);
    wait_catalog(&registry, &[spec.clone()]).await;
    let new = registry.create(request("fixture")).await.unwrap();
    assert!(directory.path().join("new.pid").is_file());
    assert_eq!(
        new.meta()
            .acp
            .unwrap()
            .modes
            .unwrap()
            .current_mode_id
            .to_string(),
        "code"
    );
    assert_eq!(
        old.meta()
            .acp
            .unwrap()
            .modes
            .unwrap()
            .current_mode_id
            .to_string(),
        "ask"
    );

    for invalid in [Some("not JSON"), None] {
        if let Some(invalid) = invalid {
            std::fs::write(&path, invalid).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(
            registry.catalog().spec(&"fixture".into()).unwrap().as_ref(),
            &spec
        );
    }
    // Remove the configured ACP agent entirely; both existing processes stay owned.
    write_settings(&path, vec![AgentSpec::embedded()]);
    wait_catalog(&registry, &[AgentSpec::embedded()]).await;
    assert!(registry.create(request("fixture")).await.is_err());
    assert!(registry.get(old.id()).await.is_ok());
    registry.create(request("embedded")).await.unwrap();
    for session in [&old, &new] {
        let mut attachment = session
            .attach(AttachOptions {
                replay: 0,
                cursor: None,
            })
            .await
            .unwrap();
        session.prompt(vec!["still alive".into()]).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let EventKind::TurnFinished { result, .. } =
                    attachment.recv().await.unwrap().kind
                {
                    assert!(result.is_ok());
                    break;
                }
            }
        })
        .await
        .expect("old backend keeps working");
    }
    stopping.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        watcher.await.unwrap();
        registry.shutdown().await;
    })
    .await
    .expect("watcher and ACP sessions stop");
}
