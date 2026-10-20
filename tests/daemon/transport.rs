use super::*;

fn local_target(directory: &std::path::Path) -> Target {
    #[cfg(unix)]
    {
        Target::Unix(directory.join("ipc/service.sock"))
    }
    #[cfg(windows)]
    {
        let _ = directory;
        Target::Pipe(format!("bas-test-{}", uuid::Uuid::new_v4()))
    }
}

fn local_command(target: &Target) -> Command {
    let mut command = command();
    command.args([
        "--listen",
        &target.to_string(),
        "--notify",
        "never",
        "--log-backend",
        "stdout",
        "--log-filter",
        "info",
    ]);
    command
}

async fn cli(target: Option<&Target>, args: &[&str], runtime: &std::path::Path) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bast"));
    command
        .env_remove("BAS_SERVER")
        .env("XDG_RUNTIME_DIR", runtime)
        .kill_on_drop(true);
    if let Some(target) = target {
        command.arg("--server").arg(target.to_string());
    }
    let output = tokio::time::timeout(TIMEOUT, command.args(args).output())
        .await
        .expect("bounded CLI")
        .expect("CLI starts");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 output")
}

#[tokio::test]
async fn local_ipc_supports_rpc_cli_and_local_management_without_pairing() {
    let directory = tempfile::tempdir().expect("private directory");
    let target = local_target(directory.path());
    let mut daemon = Daemon::start(local_command(&target)).await;
    assert_eq!(daemon.address, target);

    let agents = daemon
        .client
        .list_agents(pb::ListAgentsRequest {})
        .await
        .expect("local RPC")
        .into_inner();
    assert!(agents.agents.iter().any(|agent| agent.id == "embedded"));
    assert!(
        cli(Some(&target), &["agents"], directory.path())
            .await
            .contains("embedded")
    );

    // Removing a nonexistent registration is a local-management RPC. It may
    // fail for application reasons, but must not be rejected as a remote peer.
    let error = daemon
        .client
        .remove_agent(pb::RemoveAgentRequest {
            id: "transport-test-does-not-exist".into(),
        })
        .await
        .expect_err("no writable agent registration");
    assert_ne!(error.code(), tonic::Code::PermissionDenied);
    daemon.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn default_daemon_and_cli_use_the_same_local_endpoint() {
    let directory = tempfile::tempdir().expect("runtime directory");
    let mut command = command();
    command.env("XDG_RUNTIME_DIR", directory.path()).args([
        "--notify",
        "never",
        "--log-backend",
        "stdout",
        "--log-filter",
        "info",
    ]);
    let mut daemon = Daemon::start(command).await;
    assert!(matches!(daemon.address, Target::Unix(_)));
    assert!(
        cli(None, &["agents"], directory.path())
            .await
            .contains("embedded")
    );
    let Target::Unix(path) = daemon.address.clone() else {
        unreachable!()
    };
    daemon.stop().await;
    assert!(!path.exists(), "graceful shutdown removes the owned socket");
}

#[tokio::test]
async fn localhost_tcp_resolves_at_bind_and_connect() {
    let target: Target = "tcp://localhost:0".parse().expect("hostname target");
    let mut daemon = Daemon::start(local_command(&target)).await;
    let bound: std::net::SocketAddr = daemon.address.to_string().parse().expect("actual bound IP");
    assert_ne!(bound.port(), 0);
    assert!(bound.ip().is_loopback());
    let target: Target = format!("localhost:{}", bound.port())
        .parse()
        .expect("hostname");
    assert!(!target.is_local());
    assert!(!target.is_loopback_tcp());
    let mut client = Client::connect_target(&target)
        .await
        .expect("hostname connection");
    let agents = client
        .list_agents(pb::ListAgentsRequest {})
        .await
        .expect("hostname RPC")
        .into_inner();
    assert!(agents.agents.iter().any(|agent| agent.id == "embedded"));
    let directory = tempfile::tempdir().expect("runtime directory");
    assert!(
        cli(Some(&target), &["agents"], directory.path())
            .await
            .contains("embedded")
    );
    daemon.stop().await;
}

#[tokio::test]
async fn explicit_tcp_and_local_ipc_share_one_service() {
    let directory = tempfile::tempdir().expect("private directory");
    let target = local_target(directory.path());
    let mut command = local_command(&target);
    command.args(["--listen", "tcp://127.0.0.1:0"]);
    let mut daemon = Daemon::start(command).await;
    let tcp: Target = tokio::time::timeout(TIMEOUT, async {
        loop {
            let line = daemon
                ._logs
                .next_line()
                .await
                .expect("read log")
                .expect("TCP readiness");
            if line.contains("serving the bas.v1.Bas service") {
                break line
                    .split("address=")
                    .nth(1)
                    .expect("address")
                    .split_whitespace()
                    .next()
                    .expect("target")
                    .parse()
                    .expect("TCP target");
            }
        }
    })
    .await
    .expect("TCP ready");
    assert!(matches!(tcp, Target::Tcp(_)));
    let created = cli(
        Some(&target),
        &["session", "create", "--title", "shared-transport"],
        directory.path(),
    )
    .await;
    let id = created
        .lines()
        .find_map(|line| line.strip_prefix("id        "))
        .expect("session ID");
    let listed = cli(Some(&tcp), &["session", "list"], directory.path()).await;
    assert!(listed.contains(id), "TCP and IPC reach the same registry");
    daemon.stop().await;
}
