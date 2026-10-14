//! Process tests for configuration selection and graceful shutdown with live RPCs.

use bas::service::{Client, pb};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
};

const TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(unix)]
#[path = "daemon/archive.rs"]
mod archive;

#[cfg(unix)]
#[path = "daemon/config.rs"]
mod config;

#[cfg(unix)]
#[path = "daemon/management.rs"]
mod management;

#[cfg(unix)]
#[path = "daemon/storage.rs"]
mod storage;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bas"));
    for key in [
        "BAS_LISTEN",
        "BAS_AGENTS",
        "BAS_AGENTS_FILE",
        "BAS_SETTINGS_FILE",
        "BAS_NOTIFY",
        "BAS_HISTORY",
        "BAS_STORAGE",
        "BAS_LOCALDB_PATH",
        "BAS_BUFFER",
        "BAS_STARTUP_TIMEOUT",
        "BAS_PERMISSIONS",
        "BAS_LOG_BACKEND",
        "RUST_LOG",
        "JOURNAL_STREAM",
        "NOTIFY_SOCKET",
        "WATCHDOG_USEC",
    ] {
        command.env_remove(key);
    }
    command
        .env("BAS_AGENTS", "embedded")
        .env("BAS_STORAGE", "memory")
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

struct Daemon {
    child: Child,
    _logs: Lines<BufReader<ChildStdout>>,
    startup_logs: Vec<String>,
    #[cfg(unix)]
    address: std::net::SocketAddr,
    client: Client,
}

impl Daemon {
    async fn start(mut command: Command) -> Self {
        let mut child = command.spawn().expect("daemon starts");
        let mut logs = BufReader::new(child.stdout.take().expect("stdout pipe")).lines();
        let mut startup_logs = Vec::new();
        let address = tokio::time::timeout(TIMEOUT, async {
            loop {
                let Some(line) = logs.next_line().await.expect("read daemon log") else {
                    let mut error = String::new();
                    child
                        .stderr
                        .as_mut()
                        .expect("stderr pipe")
                        .read_to_string(&mut error)
                        .await
                        .expect("read startup error");
                    panic!("daemon exited before readiness: {error}");
                };
                startup_logs.push(line.clone());
                if line.contains("serving the bas.v1.Bas service") {
                    return line
                        .split("address=")
                        .nth(1)
                        .expect("bound address")
                        .split_whitespace()
                        .next()
                        .expect("address value")
                        .parse()
                        .expect("socket address");
                }
            }
        })
        .await
        .expect("daemon becomes ready");
        let client = Client::connect_at(address)
            .await
            .expect("connect to daemon");
        Self {
            child,
            _logs: logs,
            startup_logs,
            #[cfg(unix)]
            address,
            client,
        }
    }

    #[cfg(unix)]
    async fn stop(&mut self) {
        use nix::{
            sys::signal::{Signal, kill},
            unistd::Pid,
        };
        kill(
            Pid::from_raw(self.child.id().expect("daemon running") as i32),
            Signal::SIGTERM,
        )
        .expect("send SIGTERM");
        let status = tokio::time::timeout(TIMEOUT, self.child.wait())
            .await
            .expect("shutdown is bounded")
            .expect("wait for daemon");
        assert!(status.success(), "daemon exit: {status}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn auto_logging_rejects_inherited_nonmatching_journal_streams() {
    for stream in ["", "not-a-stream", "0:0"] {
        let mut command = command();
        command.env("JOURNAL_STREAM", stream).args([
            "--listen",
            "127.0.0.1:0",
            "--notify",
            "never",
            "--log-backend",
            "auto",
        ]);
        let mut daemon = Daemon::start(command).await;
        assert!(
            daemon
                .startup_logs
                .iter()
                .all(|line| !line.contains("journald is unavailable")),
            "invalid JOURNAL_STREAM must not attempt journald: {:?}",
            daemon.startup_logs
        );
        assert!(
            daemon
                .startup_logs
                .iter()
                .any(|line| line.contains("backend=stdout")),
            "Auto should select stdout directly"
        );
        daemon.stop().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn daemon_discovers_the_user_agent_catalog() {
    let directory = tempfile::tempdir().expect("config directory");
    let config = directory.path().join("bas");
    std::fs::create_dir(&config).expect("create config directory");
    std::fs::write(
        config.join("agents.json"),
        r#"[{"id":"configured","kind":"embedded"}]"#,
    )
    .expect("write catalog");
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
    let agents = daemon
        .client
        .list_agents(pb::ListAgentsRequest {})
        .await
        .expect("agents")
        .into_inner()
        .agents;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, "configured");
    daemon.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_overrides_invalid_environment_and_shutdown_closes_live_streams() {
    let mut command = command();
    for key in [
        "BAS_LISTEN",
        "BAS_AGENTS",
        "BAS_NOTIFY",
        "BAS_HISTORY",
        "BAS_STORAGE",
        "BAS_BUFFER",
        "BAS_STARTUP_TIMEOUT",
        "BAS_PERMISSIONS",
        "BAS_LOG_BACKEND",
        "RUST_LOG",
    ] {
        command.env(key, "invalid");
    }
    command.args([
        "--listen",
        "127.0.0.1:0",
        "--agents",
        "embedded",
        "--notify",
        "never",
        "--history",
        "100",
        "--storage",
        "memory",
        "--buffer",
        "100",
        "--startup-timeout",
        "5",
        "--permissions",
        "deny",
        "--log-backend",
        "stdout",
        "--log-filter",
        "info",
    ]);
    let mut daemon = Daemon::start(command).await;
    let session = daemon
        .client
        .create_session(pb::CreateSessionRequest::default())
        .await
        .expect("create")
        .into_inner()
        .session
        .expect("session metadata");
    let mut stream = daemon
        .client
        .attach(pb::AttachRequest {
            session: session.id.clone(),
            cursor: None,
            replay: 0,
        })
        .await
        .expect("attach")
        .into_inner();
    daemon
        .client
        .prompt(pb::PromptSubmission::from(
            &bas::model::PromptSubmission::new(bas::acp::PromptRequest::new(
                session.id,
                vec![bas::acp::ContentBlock::from("long turn".repeat(100))],
            )),
        ))
        .await
        .expect("prompt");
    daemon.stop().await;
    let mut stopped = false;
    tokio::time::timeout(TIMEOUT, async {
        while let Some(event) = stream.message().await.expect("stream drains cleanly") {
            if matches!(event.kind, Some(pb::event::Kind::StatusChanged(pb::StatusChanged { status })) if status == pb::SessionStatus::Stopped as i32) { stopped = true; }
        }
    }).await.expect("stream closes");
    assert!(stopped, "shutdown must publish a stopped event");
}

#[tokio::test]
async fn zero_capacities_and_empty_catalog_are_rejected() {
    for args in [
        vec!["--history", "0"],
        vec!["--buffer", "0"],
        vec!["--startup-timeout", "0"],
        vec!["--agents", ""],
    ] {
        let output = tokio::time::timeout(TIMEOUT, command().args(args).output())
            .await
            .expect("configuration check completes")
            .expect("daemon command");
        assert!(!output.status.success());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn required_notifications_fail_startup_while_auto_allows_headless_use() {
    let mut required = command();
    required
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/bas-test-bus",
        )
        .args(["--notify", "always", "--listen", "127.0.0.1:0"]);
    let output = tokio::time::timeout(TIMEOUT, required.output())
        .await
        .expect("startup check completes")
        .expect("daemon command");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("required"));
    let mut automatic = command();
    automatic
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/bas-test-bus",
        )
        .args([
            "--notify",
            "auto",
            "--listen",
            "127.0.0.1:0",
            "--log-backend",
            "stdout",
        ]);
    Daemon::start(automatic).await.stop().await;
}
