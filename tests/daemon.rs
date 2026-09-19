//! Process tests for configuration selection and graceful shutdown with live RPCs.

use basr::service::{Client, pb};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
};

const TIMEOUT: Duration = Duration::from_secs(10);

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_basrd"));
    for key in [
        "BASRD_LISTEN",
        "BASRD_AGENTS",
        "BASRD_AGENTS_FILE",
        "BASRD_NOTIFY",
        "BASRD_HISTORY",
        "BASRD_BUFFER",
        "BASRD_STARTUP_TIMEOUT",
        "BASRD_PERMISSIONS",
        "BASRD_LOG_BACKEND",
        "RUST_LOG",
        "NOTIFY_SOCKET",
        "WATCHDOG_USEC",
    ] {
        command.env_remove(key);
    }
    command
        .env("BASRD_AGENTS", "embedded")
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

struct Daemon {
    child: Child,
    _logs: Lines<BufReader<ChildStdout>>,
    client: Client,
}

impl Daemon {
    async fn start(mut command: Command) -> Self {
        let mut child = command.spawn().expect("daemon starts");
        let mut logs = BufReader::new(child.stdout.take().expect("stdout pipe")).lines();
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
                if line.contains("serving the basr.v1.Basr service") {
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
async fn daemon_discovers_the_user_agent_catalog() {
    let directory = tempfile::tempdir().expect("config directory");
    let config = directory.path().join("basr");
    std::fs::create_dir(&config).expect("create config directory");
    std::fs::write(
        config.join("agents.json"),
        r#"[{"id":"configured","kind":"embedded"}]"#,
    )
    .expect("write catalog");
    let mut command = command();
    command
        .env_remove("BASRD_AGENTS")
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
        "BASRD_LISTEN",
        "BASRD_AGENTS",
        "BASRD_NOTIFY",
        "BASRD_HISTORY",
        "BASRD_BUFFER",
        "BASRD_STARTUP_TIMEOUT",
        "BASRD_PERMISSIONS",
        "BASRD_LOG_BACKEND",
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
        .prompt(pb::AcpPayload::from(&basr::acp::PromptRequest::new(
            session.id,
            vec![basr::acp::ContentBlock::from("long turn".repeat(100))],
        )))
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
            "unix:path=/nonexistent/basr-test-bus",
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
            "unix:path=/nonexistent/basr-test-bus",
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
