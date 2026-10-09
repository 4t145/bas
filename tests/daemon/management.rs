//! Exercise discovery, acquisition and lifecycle through the real daemon RPCs.

use super::*;
use basr::{
    config::AgentSpec,
    management::{Catalog, Status},
};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
use tokio_util::task::AbortOnDropHandle;

struct Fixture {
    address: SocketAddr,
    index: Arc<Mutex<serde_json::Value>>,
    _server: AbortOnDropHandle<()>,
}

impl Fixture {
    async fn new(directory: &std::path::Path) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener");
        let address = listener.local_addr().expect("fixture address");
        let shell = which::which("sh").expect("test shell");
        let script = format!(
            "#!{}\n{}",
            shell.display(),
            include_str!("../../src/bin/basrd/agent/acp/assets/fixture.sh")
        )
        .into_bytes();
        let digest = format!("{:x}", Sha256::digest(&script));
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let manifest = |id: &str, archive: &str, checksum: &str| {
            serde_json::json!({
                "id": id, "name": id, "version": "1.0.0", "description": "Local test ACP agent",
                "distribution": {"binary": {platform.clone(): {
                    "archive": format!("http://{address}/{archive}"), "cmd": "./agent",
                    "sha256": checksum, "env": {"BASR_FIXTURE_MODE": "normal", "BASR_FIXTURE_PID": directory.join(format!("{id}.pid"))}
                }}}
            })
        };
        let index = Arc::new(Mutex::new(serde_json::json!({
            "version": "1.0.0", "agents": [
                manifest("test-agent", "agent", &digest),
                manifest("bad-agent", "agent", &"0".repeat(64)),
                manifest("slow-agent", "slow", &digest),
                {"id":"python-agent", "name":"Python agent", "version":"1.0.0", "distribution":{"uvx":{"package":"fixture==1.0.0"}}}
            ]
        })));
        let state = index.clone();
        let server = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let state = state.clone();
                let script = script.clone();
                clients.spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let count = stream.read(&mut chunk).await.expect("fixture request");
                        if count == 0 { return; }
                        request.extend_from_slice(&chunk[..count]);
                    }
                    let request = String::from_utf8_lossy(&request);
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    if path == "/slow" {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\nx").await;
                        let _ = stream.read(&mut chunk).await;
                        return;
                    }
                    let bytes = if path == "/registry.json" {
                        serde_json::to_vec(&*state.lock().await).expect("registry fixture")
                    } else { script };
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&bytes).await;
                });
            }
        });
        Self {
            address,
            index,
            _server: AbortOnDropHandle::new(server),
        }
    }
}

async fn start(directory: &std::path::Path, fixture: &Fixture) -> Daemon {
    let path = directory.join("settings.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "appearance": {"theme_source":"builtin"}, "daemon": {"agents": [
                {"id":"embedded","kind":"embedded"},
                {"id":"missing","kind":"acp","command":{"program":directory.join("not-installed")}}
            ]}
        })
        .to_string(),
    )
    .expect("settings fixture");
    let mut command = command();
    command
        .env_remove("BASRD_AGENTS")
        .env("BASR_SETTINGS_FILE", path)
        .env("BASRD_ACP_DATA_DIR", directory.join("acp"))
        .env(
            "BASRD_ACP_REGISTRY_URL",
            format!("http://{}/registry.json", fixture.address),
        )
        .args([
            "--listen",
            "127.0.0.1:0",
            "--notify",
            "never",
            "--log-backend",
            "stdout",
        ]);
    Daemon::start(command).await
}

async fn catalog(daemon: &mut Daemon, refresh: bool) -> Catalog {
    let response = daemon
        .client
        .list_agent_catalog(pb::ListAgentCatalogRequest { refresh })
        .await
        .expect("catalog RPC")
        .into_inner();
    serde_json::from_slice(&response.catalog_json).expect("typed catalog")
}

async fn install(daemon: &mut Daemon, id: &str) -> AgentSpec {
    let mut stream = daemon
        .client
        .install_agent(pb::InstallAgentRequest { id: id.into() })
        .await
        .expect("install starts")
        .into_inner();
    tokio::time::timeout(TIMEOUT, async {
        while let Some(update) = stream.message().await.expect("install update") {
            if update.complete {
                return serde_json::from_slice(&update.spec_json).expect("registered spec");
            }
        }
        panic!("installation must finish with a saved spec");
    })
    .await
    .expect("local install deadline")
}

#[tokio::test]
async fn discovery_install_update_remove_and_offline_cache_preserve_sessions() {
    let directory = tempfile::tempdir().expect("isolated management directory");
    let fixture = Fixture::new(directory.path()).await;
    let mut daemon = start(directory.path(), &fixture).await;
    let discovered = catalog(&mut daemon, false).await;
    assert!(discovered.can_install);
    assert!(
        discovered
            .entries
            .iter()
            .any(|entry| entry.id == "missing" && entry.status == Status::Missing)
    );
    assert!(
        discovered
            .entries
            .iter()
            .any(|entry| entry.id == "python-agent" && entry.status == Status::Unsupported)
    );
    let first = install(&mut daemon, "test-agent").await;
    let checked = daemon
        .client
        .check_agent(pb::CheckAgentRequest {
            agent: "test-agent".into(),
            expected_spec_json: serde_json::to_vec(&first).expect("spec"),
            connect: true,
        })
        .await
        .expect("installed agent connects")
        .into_inner();
    assert_eq!(checked.status(), pb::AgentCheckStatus::Connected);
    let live = daemon
        .client
        .create_session(pb::CreateSessionRequest {
            agent: "test-agent".into(),
            ..Default::default()
        })
        .await
        .expect("managed session")
        .into_inner()
        .session
        .expect("session");
    fixture.index.lock().await["agents"][0]["version"] = "2.0.0".into();
    catalog(&mut daemon, true).await;
    let updated = install(&mut daemon, "test-agent").await;
    assert_ne!(
        first.command, updated.command,
        "versions have independent launch paths"
    );
    assert!(
        first
            .command
            .as_ref()
            .expect("first program")
            .program
            .is_file()
    );
    daemon
        .client
        .remove_agent(pb::RemoveAgentRequest {
            id: "test-agent".into(),
        })
        .await
        .expect("managed removal");
    daemon
        .client
        .prompt(pb::AcpPayload::from(&basr::acp::PromptRequest::new(
            live.id,
            vec!["still alive".into()],
        )))
        .await
        .expect("existing process continues after update and removal");
    let saved = basr::config::Store::at(directory.path().join("settings.json"))
        .load()
        .expect("saved settings");
    assert_eq!(
        saved.settings.appearance.theme_source,
        basr::config::appearance::ThemeSource::Builtin
    );
    assert_eq!(saved.settings.daemon.agents.len(), 2);
    assert!(
        updated.command.expect("updated program").program.is_file(),
        "removal retains version directories"
    );
    drop(fixture);
    let offline = catalog(&mut daemon, true).await;
    assert!(offline.warning.is_some());
    assert!(offline.entries.iter().any(|entry| entry.id == "test-agent"));
    daemon.stop().await;
}

#[tokio::test]
async fn checksum_failure_and_cancelled_download_never_register_a_backend() {
    let directory = tempfile::tempdir().expect("isolated management directory");
    let fixture = Fixture::new(directory.path()).await;
    let mut daemon = start(directory.path(), &fixture).await;
    let original = std::fs::read(directory.path().join("settings.json")).expect("original config");
    let mut bad = daemon
        .client
        .install_agent(pb::InstallAgentRequest {
            id: "bad-agent".into(),
        })
        .await
        .expect("bad install starts")
        .into_inner();
    let error = loop {
        match bad.message().await {
            Err(error) => break error,
            Ok(Some(update)) => assert!(!update.complete),
            Ok(None) => panic!("checksum failure must be reported"),
        }
    };
    assert!(error.message().contains("checksum"));
    let mut slow = daemon
        .client
        .install_agent(pb::InstallAgentRequest {
            id: "slow-agent".into(),
        })
        .await
        .expect("slow install starts")
        .into_inner();
    while let Some(update) = slow.message().await.expect("progress") {
        if update.phase == "downloading" {
            break;
        }
    }
    drop(slow);
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let versions = directory.path().join("acp/agents/slow-agent");
            if !versions.exists()
                || std::fs::read_dir(&versions)
                    .expect("version staging")
                    .next()
                    .is_none()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cancel cleans incomplete installation");
    assert_eq!(
        std::fs::read(directory.path().join("settings.json")).expect("preserved config"),
        original
    );
    daemon.stop().await;
}
