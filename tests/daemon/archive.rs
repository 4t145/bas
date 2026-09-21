use super::*;

impl Daemon {
    async fn cli(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_basrctl"));
        command
            .kill_on_drop(true)
            .arg("--server")
            .arg(self.address.to_string())
            .args(args);
        tokio::time::timeout(TIMEOUT, command.output())
            .await
            .expect("CLI request has a deadline")
            .expect("CLI process starts")
    }
}

fn successful(output: std::process::Output) -> String {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("CLI uses UTF-8")
}

#[tokio::test]
async fn cli_archive_and_restore_keep_the_same_conversation() {
    let mut command = command();
    command.args([
        "--listen",
        "127.0.0.1:0",
        "--notify",
        "never",
        "--log-backend",
        "stdout",
        "--log-filter",
        "info",
    ]);
    let mut daemon = Daemon::start(command).await;
    let created = successful(
        daemon
            .cli(&["session", "create", "--title", "archive regression"])
            .await,
    );
    let id = created
        .lines()
        .find_map(|line| line.strip_prefix("id        "))
        .expect("create prints a session ID")
        .to_owned();
    let mut stream = daemon
        .client
        .attach(pb::AttachRequest {
            session: id.clone(),
            cursor: None,
            replay: 0,
        })
        .await
        .expect("attach")
        .into_inner();
    successful(daemon.cli(&["prompt", &id, "before archive"]).await);
    tokio::time::timeout(TIMEOUT, async {
        while let Some(event) = stream.message().await.expect("event stream") {
            if matches!(event.kind, Some(pb::event::Kind::TurnFinished(_))) {
                return;
            }
        }
        panic!("turn did not complete");
    })
    .await
    .expect("embedded turn completes");
    let original_history = successful(daemon.cli(&["history", &id]).await);
    assert!(original_history.contains("before archive"));

    let archived = successful(daemon.cli(&["session", "archive", &id]).await);
    assert!(archived.contains("archived  true"));
    assert!(!successful(daemon.cli(&["session", "list"]).await).contains(&id));
    let all = successful(daemon.cli(&["session", "list", "--all"]).await);
    assert!(all.contains(&id) && all.contains("[archived]"));
    let history = successful(daemon.cli(&["history", &id]).await);
    assert!(history.starts_with(&original_history));
    assert!(history.contains("archived"));
    let blocked = daemon.cli(&["prompt", &id, "blocked"]).await;
    assert!(!blocked.status.success(), "archived prompts must fail");
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("archived"));
    assert!(successful(daemon.cli(&["session", "show", &id]).await).contains("archived  true"));

    let restored = successful(daemon.cli(&["session", "restore", &id]).await);
    assert!(restored.contains(&id) && restored.contains("archived  false"));
    assert!(successful(daemon.cli(&["session", "list"]).await).contains(&id));
    successful(daemon.cli(&["prompt", &id, "after restore"]).await);
    successful(daemon.cli(&["session", "remove", &id]).await);
    assert!(!daemon.cli(&["session", "show", &id]).await.status.success());
    drop(stream);
    daemon.stop().await;
}
