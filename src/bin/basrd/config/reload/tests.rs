use super::*;
use crate::{
    agent::{Catalog, Command, LocalHost},
    session::{AttachOptions, CreateRequest, Limits},
};
use basr::model::{EventKind, Owner};

fn write_settings(path: &std::path::Path, agents: Vec<AgentSpec>) {
    let mut settings = basr::config::Settings::default();
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

#[test]
fn explicit_settings_never_fall_back_after_invalid_or_missing_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    std::fs::write(
        directory.path().join("agents.json"),
        r#"[{"id":"legacy","kind":"embedded"}]"#,
    )
    .unwrap();
    let mut source = CatalogSource::settings(Some(path.clone())).unwrap();
    assert!(source.load().is_err());
    write_settings(&path, vec![AgentSpec::embedded()]);
    assert_eq!(source.load().unwrap(), vec![AgentSpec::embedded()]);
    for invalid in [
        "not json",
        r#"{"daemon":{"agents":[]}}"#,
        r#"{"daemon":{"agents":[{"id":"a","kind":"embedded"},{"id":"a","kind":"embedded"}]}}"#,
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(source.load().is_err());
    }
    std::fs::remove_file(path).unwrap();
    assert!(source.load().is_err());
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
    spec.env.insert("BASR_FIXTURE_MODE".into(), "normal".into());
    spec.env.insert(
        "BASR_FIXTURE_PID".into(),
        directory
            .path()
            .join("old.pid")
            .to_string_lossy()
            .into_owned(),
    );
    write_settings(&path, vec![spec.clone()]);
    let mut source = CatalogSource::settings(Some(path.clone())).unwrap();
    let agents = source.load().unwrap();
    let registry = Arc::new(Registry::new(
        Arc::new(Catalog::from_specs(agents.clone()).unwrap()),
        Arc::new(LocalHost::default()),
        Limits::default(),
    ));
    let old = registry.create(request("fixture")).await.unwrap();
    let stopping = CancellationToken::new();
    let watcher = watch_catalog(source, agents, Arc::clone(&registry), stopping.clone());

    // Same id, new environment: the actual new process must receive the new spec.
    spec.env
        .insert("BASR_FIXTURE_MODE".into(), "early_update".into());
    spec.env.insert(
        "BASR_FIXTURE_PID".into(),
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
