use super::*;
use std::path::Path;

fn write_settings(path: &Path, id: &str) {
    std::fs::write(
        path,
        serde_json::json!({
            "daemon": { "agents": [{ "id": id, "kind": "embedded" }] }
        })
        .to_string(),
    )
    .unwrap();
}

async fn ids(daemon: &mut Daemon) -> Vec<String> {
    daemon
        .client
        .list_agents(pb::ListAgentsRequest {})
        .await
        .unwrap()
        .into_inner()
        .agents
        .into_iter()
        .map(|agent| agent.id)
        .collect()
}

async fn wait_id(daemon: &mut Daemon, expected: &str) {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if ids(daemon).await == [expected] {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("ListAgents sees reload");
}

#[tokio::test]
async fn settings_file_environment_reloads_catalog_and_preserves_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    write_settings(&path, "before");
    let mut command = command();
    command
        .env_remove("BAS_AGENTS")
        .env("BAS_SETTINGS_FILE", &path)
        .args([
            "--listen",
            "127.0.0.1:0",
            "--notify",
            "never",
            "--log-backend",
            "stdout",
        ]);
    let mut daemon = Daemon::start(command).await;
    assert_eq!(ids(&mut daemon).await, ["before"]);
    let old = daemon
        .client
        .create_session(pb::CreateSessionRequest {
            agent: "before".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .session
        .unwrap();
    write_settings(&path, "after");
    wait_id(&mut daemon, "after").await;
    daemon
        .client
        .create_session(pb::CreateSessionRequest {
            agent: "after".into(),
            ..Default::default()
        })
        .await
        .expect("new configured agent starts");
    daemon
        .client
        .get_session(pb::GetSessionRequest {
            session: old.id.clone(),
        })
        .await
        .expect("removed agent's session remains");
    daemon
        .client
        .prompt(pb::PromptSubmission::from(
            &bas::model::PromptSubmission::new(bas::acp::PromptRequest::new(
                old.id,
                vec!["still alive".into()],
            )),
        ))
        .await
        .expect("old session still accepts work");
    daemon.stop().await;
}

#[tokio::test]
async fn explicit_agents_override_files_and_do_not_watch_them() {
    let directory = tempfile::tempdir().unwrap();
    let settings = directory.path().join("settings.json");
    let legacy = directory.path().join("agents.json");
    std::fs::write(&settings, "invalid").unwrap();
    std::fs::write(&legacy, "invalid").unwrap();
    let mut command = command();
    command
        .args(["--agents", "fixed=embedded", "--settings-file"])
        .arg(&settings)
        .arg("--agents-file")
        .arg(&legacy)
        .args([
            "--listen",
            "127.0.0.1:0",
            "--notify",
            "never",
            "--log-backend",
            "stdout",
        ]);
    let mut daemon = Daemon::start(command).await;
    assert_eq!(ids(&mut daemon).await, ["fixed"]);
    write_settings(&settings, "changed");
    std::fs::write(&legacy, r#"[{"id":"changed","kind":"embedded"}]"#).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(ids(&mut daemon).await, ["fixed"]);
    daemon.stop().await;
}

#[tokio::test]
async fn explicit_legacy_file_precedes_settings_and_reloads() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = directory.path().join("agents.json");
    std::fs::write(&legacy, r#"[{"id":"legacy","kind":"embedded"}]"#).unwrap();
    let mut command = command();
    command
        .env_remove("BAS_AGENTS")
        .env(
            "BAS_SETTINGS_FILE",
            directory.path().join("missing-settings.json"),
        )
        .arg("--agents-file")
        .arg(&legacy)
        .args([
            "--listen",
            "127.0.0.1:0",
            "--notify",
            "never",
            "--log-backend",
            "stdout",
        ]);
    let mut daemon = Daemon::start(command).await;
    assert_eq!(ids(&mut daemon).await, ["legacy"]);
    std::fs::write(&legacy, r#"[{"id":"reloaded","kind":"embedded"}]"#).unwrap();
    wait_id(&mut daemon, "reloaded").await;
    daemon.stop().await;
}

#[tokio::test]
async fn missing_explicit_settings_fail_instead_of_starting_embedded() {
    let directory = tempfile::tempdir().unwrap();
    let output = command()
        .env_remove("BAS_AGENTS")
        .arg("--settings-file")
        .arg(directory.path().join("missing.json"))
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
}

#[tokio::test]
async fn default_catalog_migrates_once_without_falling_back_on_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("bas");
    std::fs::create_dir(&config).unwrap();
    let legacy = config.join("agents.json");
    let settings = config.join("settings.json");
    std::fs::write(&legacy, r#"[{"id":"legacy","kind":"embedded"}]"#).unwrap();
    let mut command = command();
    command
        .env_remove("BAS_AGENTS")
        .env("XDG_CONFIG_HOME", directory.path())
        .args([
            "--listen",
            "127.0.0.1:0",
            "--notify",
            "never",
            "--log-backend",
            "stdout",
        ]);
    let mut daemon = Daemon::start(command).await;
    assert_eq!(ids(&mut daemon).await, ["legacy"]);
    assert!(!settings.exists(), "legacy import is read-only");
    std::fs::remove_file(&legacy).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(ids(&mut daemon).await, ["legacy"]);
    write_settings(&settings, "unified");
    wait_id(&mut daemon, "unified").await;
    std::fs::write(&legacy, r#"[{"id":"resurrected","kind":"embedded"}]"#).unwrap();
    std::fs::remove_file(&settings).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(ids(&mut daemon).await, ["unified"]);
    daemon.stop().await;
}

#[tokio::test]
async fn reload_errors_are_deduplicated_without_logging_configuration_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    write_settings(&path, "valid");
    let mut command = command();
    command
        .env_remove("BAS_AGENTS")
        .arg("--settings-file")
        .arg(&path)
        .args([
            "--listen",
            "127.0.0.1:0",
            "--notify",
            "never",
            "--log-backend",
            "stdout",
        ]);
    let mut daemon = Daemon::start(command).await;
    std::fs::write(
        &path,
        r#"{"daemon":{"agents":[{"id":"a","kind":"sensitive-config-value"}]}}"#,
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert_eq!(ids(&mut daemon).await, ["valid"]);
    let mut errors = 0;
    while let Ok(Some(line)) =
        tokio::time::timeout(Duration::from_millis(100), daemon._logs.next_line())
            .await
            .map(|result| result.unwrap())
    {
        assert!(!line.contains("sensitive-config-value"));
        if line.contains("cannot reload agent catalog") {
            errors += 1;
        }
    }
    assert_eq!(errors, 1, "one warning per consecutive identical error");
    daemon.stop().await;
}
