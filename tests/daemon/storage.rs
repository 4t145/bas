use super::*;

#[tokio::test]
async fn storage_failure_can_be_retried_without_restarting_daemon_or_resending_prompt() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("isolated database");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private directory");
    let database = directory.path().join("sessions.sqlite3");
    let mut process = command();
    process
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
    let mut daemon = Daemon::start(process).await;
    let created = daemon
        .client
        .create_session(pb::CreateSessionRequest::default())
        .await
        .expect("create")
        .into_inner()
        .session
        .expect("snapshot");
    let id = created.id.clone();
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
    let turn = bas::model::TurnId::new().to_string();
    let prompt = [
        "prompt",
        &id,
        "--submission",
        &turn,
        "committed before storage failure",
    ];
    cli(&daemon, &prompt).await;
    tokio::time::timeout(TIMEOUT, async {
        while let Some(event) = stream.message().await.expect("turn event") {
            if matches!(event.kind, Some(pb::event::Kind::TurnFinished(_))) {
                return;
            }
        }
        panic!("missing completion");
    })
    .await
    .expect("completed before fault injection");
    let before = cli(&daemon, &["history", &id]).await;
    // Inject a real SQLite commit failure without filling the user's disk.
    let writer = rusqlite::Connection::open(&database).expect("fault injector");
    writer
        .execute_batch(
            "CREATE TRIGGER storage_unavailable BEFORE INSERT ON history_chunks
         BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
        )
        .expect("inject failure");
    assert!(
        daemon
            .client
            .set_title(pb::SetTitleRequest {
                session: id.clone(),
                title: "not committed".into(),
            })
            .await
            .is_err()
    );
    let request = pb::GetSessionRequest {
        session: id.clone(),
    };
    let failed = daemon
        .client
        .get_session(request.clone())
        .await
        .expect("failed snapshot")
        .into_inner()
        .session
        .expect("session")
        .runtime
        .expect("runtime");
    assert!(failed.failure.is_some());
    let retry = cli_output(&daemon, &["session", "recover", &id]).await;
    assert!(
        !retry.status.success(),
        "unwritable database remains failed"
    );
    assert_eq!(cli(&daemon, &["history", &id]).await, before);
    writer
        .execute_batch("DROP TRIGGER storage_unavailable")
        .expect("restore writes");
    cli(&daemon, &["session", "recover", &id]).await;
    let recovered = daemon
        .client
        .get_session(request)
        .await
        .expect("recovered snapshot")
        .into_inner()
        .session
        .expect("session");
    let runtime = recovered.runtime.expect("runtime");
    assert_ne!(runtime.id, failed.id);
    assert!(runtime.failure.is_none());
    assert_eq!(recovered.id, id);
    assert_ne!(recovered.title.as_deref(), Some("not committed"));
    assert_eq!(
        cli(&daemon, &["prompt-status", &id, &turn]).await,
        format!("turn {turn}\n")
    );
    let recovered_history = cli(&daemon, &["history", &id]).await;
    assert!(recovered_history.starts_with(&before));
    assert_eq!(cli(&daemon, &prompt).await, format!("turn {turn}\n"));
    assert_eq!(
        cli(&daemon, &["history", &id]).await,
        recovered_history,
        "duplicate stays read-only"
    );
    cli(
        &daemon,
        &["prompt", &id, "new work after explicit recovery"],
    )
    .await;
    daemon.stop().await;
}

async fn cli_output(daemon: &Daemon, args: &[&str]) -> std::process::Output {
    tokio::time::timeout(
        TIMEOUT,
        Command::new(env!("CARGO_BIN_EXE_bast"))
            .kill_on_drop(true)
            .arg("--server")
            .arg(daemon.address.to_string())
            .args(args)
            .output(),
    )
    .await
    .expect("CLI request deadline")
    .expect("run CLI")
}

async fn cli(daemon: &Daemon, args: &[&str]) -> String {
    let output = cli_output(daemon, args).await;
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
    let database = directory.path().join("bas/sessions.sqlite3");
    let start = || {
        let mut command = command();
        command
            .env_remove("BAS_STORAGE")
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
    let inspector = rusqlite::Connection::open(&database).expect("inspect isolated test database");
    let blocks: i64 = inspector
        .query_row(
            "SELECT COUNT(*) FROM history_chunks WHERE session_id = ?1 AND first_cursor < last_cursor",
            [&id],
            |row| row.get(0),
        )
        .expect("completed turn blocks");
    assert!(blocks > 0, "the real daemon packs completed turns");
    drop(inspector);
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
    let turn = bas::model::TurnId::new().to_string();
    let before = cli(&daemon, &["history", &id]).await;
    let unknown = cli(&daemon, &["prompt-status", &id, &turn]).await;
    assert!(unknown.contains("acceptance unknown"));
    assert_eq!(cli(&daemon, &["history", &id]).await, before);
    let prompt = [
        "prompt",
        &id,
        "--submission",
        &turn,
        "accepted before crash",
    ];
    let accepted = cli_output(&daemon, &prompt).await;
    assert!(accepted.status.success());
    assert_eq!(
        String::from_utf8_lossy(&accepted.stdout),
        format!("turn {turn}\n")
    );
    assert!(String::from_utf8_lossy(&accepted.stderr).contains(&format!("submission {turn}")));
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
    let history = cli(&daemon, &["history", &id]).await;
    assert!(history.contains("accepted before crash"));
    assert_eq!(
        cli(&daemon, &["prompt-status", &id, &turn]).await,
        format!("turn {turn}\n")
    );
    assert_eq!(cli(&daemon, &prompt).await, format!("turn {turn}\n"));
    let conflict = cli_output(
        &daemon,
        &["prompt", &id, "--submission", &turn, "different text"],
    )
    .await;
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("prompt rejected (Conflict)"));
    assert_eq!(cli(&daemon, &["history", &id]).await, history);
    cli(&daemon, &["prompt", &id, "continued after crash"]).await;
    daemon.stop().await;
}
