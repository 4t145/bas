use super::*;

fn spec(directory: &tempfile::TempDir, mode: &str) -> Arc<AgentSpec> {
    let mut spec = fixture_spec(mode, directory);
    spec.command = Some(Command::new(
        "sh",
        [
            "-c".to_owned(),
            include_str!("../assets/prewarm.sh").to_owned(),
        ],
    ));
    Arc::new(spec)
}

fn launch(spec: Arc<AgentSpec>, updates: mpsc::Sender<Update>) -> LaunchRequest {
    LaunchRequest {
        session: SessionId::new(),
        spec,
        cwd: std::env::current_dir().expect("cwd"),
        additional_directories: Vec::new(),
        host: Arc::new(LocalHost::default()),
        updates,
    }
}

fn events(directory: &tempfile::TempDir) -> String {
    std::fs::read_to_string(directory.path().join("pid.events")).expect("fixture events")
}

#[tokio::test]
async fn a_dead_unbound_process_is_not_a_warm_connection() {
    let directory = tempfile::tempdir().expect("directory");
    let warm = Warm::initialize(spec(&directory, "create"))
        .await
        .expect("initialize");
    let pid: i32 = std::fs::read_to_string(directory.path().join("pid"))
        .expect("pid")
        .trim()
        .parse()
        .expect("numeric pid");
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("kill fixture");
    tokio::time::timeout(TIMEOUT, async {
        while warm.is_alive() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dead warm connection is detected");
    warm.shutdown().await;
}

#[tokio::test]
async fn initialization_is_unbound_and_checkout_consumes_the_same_process() {
    let directory = tempfile::tempdir().expect("directory");
    let spec = spec(&directory, "create");
    let warm = Warm::initialize(spec.clone()).await.expect("initialize");
    let pid = std::fs::read_to_string(directory.path().join("pid")).expect("pid");
    assert_eq!(events(&directory), "initialize\n");
    assert!(warm.state.lock().await.session.is_none());
    let denied = std::fs::read_to_string(directory.path().join("pid.denied")).expect("denial");
    assert!(denied.contains("-32602"));

    let (updates, mut received) = mpsc::channel(16);
    let mut session = warm
        .bind(launch(spec, updates), Startup::Create)
        .await
        .expect("bind");
    assert_eq!(
        session.initialization_raw().expect("initialize")["futureInitialize"],
        17
    );
    assert!(
        received.try_recv().is_err(),
        "warm notifications must not become history"
    );
    assert_eq!(events(&directory), "initialize\nnew\n");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("pid")).expect("pid"),
        pid
    );
    assert!(!session.try_idle_reclaim(Duration::from_secs(60)));
    assert!(session.try_idle_reclaim(Duration::ZERO));
    assert!(!session.try_idle_reclaim(Duration::ZERO));
    tokio::join!(session.drain_updates(), async {
        let Some(Update::Barrier(committed)) = received.recv().await else {
            panic!("sealed transport drains through a barrier");
        };
        committed.send(()).expect("drainer waits");
    });
    session.release().await;
    assert_eq!(
        events(&directory),
        "initialize\nnew\n",
        "release must not close"
    );
    wait_for_exit(&directory).await;
}

#[tokio::test]
async fn warm_restore_replays_only_load_history_and_explicit_shutdown_closes() {
    let directory = tempfile::tempdir().expect("directory");
    let spec = spec(&directory, "load");
    let warm = Warm::initialize(spec.clone()).await.expect("initialize");
    let (updates, mut received) = mpsc::channel(16);
    let mut session = warm
        .bind(launch(spec, updates), Startup::Restore("fixture".into()))
        .await
        .expect("load");
    assert!(matches!(
        received.recv().await,
        Some(Update::Acp(v1::SessionUpdate::AgentMessageChunk(_)))
    ));
    assert!(received.try_recv().is_err());
    assert_eq!(events(&directory), "initialize\nload\n");
    session.shutdown().await;
    assert_eq!(events(&directory), "initialize\nload\nclose\n");
    wait_for_exit(&directory).await;
}

#[tokio::test]
async fn failed_warm_create_is_not_retried() {
    let directory = tempfile::tempdir().expect("directory");
    let spec = spec(&directory, "reject");
    let warm = Warm::initialize(spec.clone()).await.expect("initialize");
    let (updates, _received) = mpsc::channel(16);
    assert!(
        warm.bind(launch(spec, updates), Startup::Create)
            .await
            .is_err()
    );
    assert_eq!(events(&directory), "initialize\nnew\n");
    wait_for_exit(&directory).await;
}
