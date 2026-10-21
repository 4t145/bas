use super::*;
use std::path::Path;

fn traffic(path: &Path, run: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with(&format!("{run} ")))
        .map(|line| line.trim().to_owned())
        .collect()
}

async fn wait_traffic(path: &Path, run: &str, expected: &str) {
    tokio::time::timeout(TIMEOUT, async {
        while !traffic(path, run).iter().any(|line| line == expected) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("expected ACP request");
}

#[tokio::test]
async fn restart_only_initializes_last_used_reserve_and_keeps_old_sessions_lazy() {
    let directory = tempfile::tempdir().expect("private state");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private state permissions");
    let database = directory.path().join("sessions.sqlite3");
    let settings = directory.path().join("settings.json");
    let log = directory.path().join("traffic");
    let agents: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "kind": "acp",
                "command": {
                    "program": "sh",
                    "args": ["-c", include_str!("acp_lifecycle/assets/restart.sh")]
                },
                "env": {"BAS_RESTART_AGENT": id, "BAS_RESTART_LOG": log}
            })
        })
        .collect();
    std::fs::write(
        &settings,
        serde_json::json!({"daemon": {"agents": agents}}).to_string(),
    )
    .expect("settings");
    let start = |run: &str| {
        let mut process = command();
        process
            .env_remove("BAS_AGENTS")
            .env("BAS_RESTART_RUN", run)
            .args([
                "--listen",
                "127.0.0.1:0",
                "--notify",
                "never",
                "--log-backend",
                "stdout",
                "--storage",
                "localdb",
                "--localdb-path",
            ])
            .arg(&database)
            .arg("--settings-file")
            .arg(&settings);
        process
    };
    let mut daemon = Daemon::start(start("1")).await;
    let mut sessions = Vec::new();
    for agent in ["a", "a", "b"] {
        sessions.push(
            daemon
                .client
                .create_session(pb::CreateSessionRequest {
                    agent: agent.into(),
                    ..Default::default()
                })
                .await
                .expect("create")
                .into_inner()
                .session
                .expect("session"),
        );
    }
    daemon
        .client
        .prompt(pb::PromptSubmission::from(
            &bas::model::PromptSubmission::new(bas::acp::PromptRequest::new(
                sessions[0].id.clone(),
                vec!["before restart".into()],
            )),
        ))
        .await
        .expect("accepted prompt selects a, not most recently created b");
    let raw_id = traffic(&log, "1")
        .into_iter()
        .find_map(|line| line.strip_prefix("1 a new ").map(str::to_owned))
        .expect("first a backend ID");
    daemon.stop().await;
    drop(daemon);

    let mut daemon = Daemon::start(start("2")).await;
    wait_traffic(&log, "2", "2 a initialize").await;
    let listed = daemon
        .client
        .list_sessions(pb::ListSessionsRequest {
            include_archived: true,
        })
        .await
        .expect("list without backend")
        .into_inner();
    assert_eq!(listed.sessions.len(), 3);
    for session in &sessions {
        daemon
            .client
            .load_history(pb::LoadHistoryRequest {
                session: session.id.clone(),
                after: None,
                limit: 100,
            })
            .await
            .expect("history without backend");
    }
    assert_eq!(
        traffic(&log, "2"),
        ["2 a initialize"],
        "restart prewarms one blank a connection; no b, session/new, load or replayed prompt"
    );

    daemon
        .client
        .prompt(pb::PromptSubmission::from(
            &bas::model::PromptSubmission::new(bas::acp::PromptRequest::new(
                sessions[0].id.clone(),
                vec!["after restart".into()],
            )),
        ))
        .await
        .expect("new prompt restores original session");
    wait_traffic(&log, "2", &format!("2 a load {raw_id}")).await;
    wait_traffic(&log, "2", "2 a prompt").await;
    assert!(
        !traffic(&log, "2")
            .iter()
            .any(|line| line.starts_with("2 a new "))
    );
    daemon.stop().await;
}
