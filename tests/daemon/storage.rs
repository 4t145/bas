use super::*;

async fn cli(daemon: &Daemon, args: &[&str]) -> String {
    let output = tokio::time::timeout(
        TIMEOUT,
        Command::new(env!("CARGO_BIN_EXE_basrctl"))
            .arg("--server")
            .arg(daemon.address.to_string())
            .args(args)
            .output(),
    )
    .await
    .expect("CLI request deadline")
    .expect("run CLI");
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("CLI UTF-8")
}

#[tokio::test]
async fn localdb_survives_daemon_restart_with_history_and_the_same_session_id() {
    let directory = tempfile::tempdir().expect("storage directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private storage directory");
    }
    let database = directory.path().join("basr/sessions.sqlite3");
    let start = || {
        let mut command = command();
        command
            .env_remove("BASRD_STORAGE")
            .env("XDG_STATE_HOME", directory.path())
            .arg("--listen")
            .arg("127.0.0.1:0")
            .arg("--notify")
            .arg("never")
            .arg("--log-backend")
            .arg("stdout");
        command
    };

    let mut daemon = Daemon::start(start()).await;
    let created = cli(&daemon, &["session", "create", "--title", "persisted"]).await;
    assert!(
        database.exists(),
        "default storage creates the local database"
    );
    let id = created
        .lines()
        .find_map(|line| line.strip_prefix("id        "))
        .expect("create prints the session ID")
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
    cli(&daemon, &["prompt", &id, "before restart"]).await;
    tokio::time::timeout(TIMEOUT, async {
        while let Some(event) = stream.message().await.expect("event stream") {
            if matches!(event.kind, Some(pb::event::Kind::TurnFinished(_))) {
                return;
            }
        }
        panic!("first turn did not complete");
    })
    .await
    .expect("first turn completes");
    cli(&daemon, &["session", "archive", &id]).await;
    daemon.stop().await;
    drop(stream);
    drop(daemon);

    let mut daemon = Daemon::start(start()).await;
    assert!(!cli(&daemon, &["session", "list"]).await.contains(&id));
    assert!(
        cli(&daemon, &["session", "list", "--all"])
            .await
            .contains(&id)
    );
    assert!(
        cli(&daemon, &["history", &id])
            .await
            .contains("before restart")
    );
    cli(&daemon, &["session", "restore", &id]).await;
    let mut stream = daemon
        .client
        .attach(pb::AttachRequest {
            session: id.clone(),
            cursor: None,
            replay: 0,
        })
        .await
        .expect("reattach")
        .into_inner();
    cli(&daemon, &["prompt", &id, "after restart"]).await;
    tokio::time::timeout(TIMEOUT, async {
        while let Some(event) = stream.message().await.expect("event stream") {
            if matches!(event.kind, Some(pb::event::Kind::TurnFinished(_))) {
                return;
            }
        }
        panic!("resumed turn did not complete");
    })
    .await
    .expect("resumed turn completes");
    let history = cli(&daemon, &["history", &id]).await;
    assert!(history.contains("before restart") && history.contains("after restart"));
    daemon.stop().await;
}

#[tokio::test]
async fn acknowledged_prompt_remains_replayable_after_daemon_is_killed() {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("storage directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private storage directory");
    let database = directory.path().join("sessions.sqlite");
    let start = || {
        let mut command = command();
        command
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
            .arg(&database);
        command
    };
    let mut daemon = Daemon::start(start()).await;
    let created = cli(&daemon, &["session", "create"]).await;
    let id = created
        .lines()
        .find_map(|line| line.strip_prefix("id        "))
        .expect("create prints the session ID")
        .to_owned();
    cli(&daemon, &["prompt", &id, "accepted before crash"]).await;
    kill(
        Pid::from_raw(daemon.child.id().expect("daemon running") as i32),
        Signal::SIGKILL,
    )
    .expect("kill daemon without graceful shutdown");
    tokio::time::timeout(TIMEOUT, daemon.child.wait())
        .await
        .expect("daemon exits")
        .expect("wait for killed daemon");
    drop(daemon);

    let mut daemon = Daemon::start(start()).await;
    assert!(cli(&daemon, &["session", "list"]).await.contains(&id));
    assert!(
        cli(&daemon, &["history", &id])
            .await
            .contains("accepted before crash")
    );
    cli(&daemon, &["prompt", &id, "continued after crash"]).await;
    daemon.stop().await;
}
