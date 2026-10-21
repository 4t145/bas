use super::*;

#[test]
fn retry_backoff_is_exponential_and_capped() {
    assert_eq!(next_backoff(INITIAL_BACKOFF), INITIAL_BACKOFF * 2);
    let mut delay = INITIAL_BACKOFF;
    for _ in 0..100 {
        delay = next_backoff(delay);
        assert!(delay <= MAX_BACKOFF);
    }
    assert_eq!(delay, MAX_BACKOFF);
    assert_eq!(next_backoff(Duration::MAX), MAX_BACKOFF);
}

#[tokio::test]
async fn capacity_is_updated_only_for_an_equal_spec() {
    let pool = Pool::default();
    let spec = Arc::new(AgentSpec::embedded());
    pool.maintain(spec.clone(), 2, Duration::from_secs(1));
    pool.maintain(Arc::new((*spec).clone()), 3, Duration::from_secs(2));
    let mut changed = (*spec).clone();
    changed.env.insert("POOL_TEST".into(), "different".into());
    pool.maintain(Arc::new(changed), 1, Duration::from_secs(1));
    {
        let state = pool.state.lock().expect("pool state");
        assert_eq!(state.entries.len(), 2);
        let first = state.entries[0].shared.state.lock().expect("inventory");
        assert_eq!(first.target, 3);
        assert_eq!(first.timeout, Duration::from_secs(2));
    }
    pool.shutdown().await;
    assert!(pool.take(&spec).is_none());
    pool.maintain(spec, 4, Duration::from_secs(1));
    assert!(pool.state.lock().expect("pool state").entries.is_empty());
    pool.shutdown().await;
}

#[tokio::test]
async fn dropping_the_pool_does_not_leave_a_worker_ownership_cycle() {
    let pool = Pool::default();
    pool.maintain(Arc::new(AgentSpec::embedded()), 1, Duration::from_secs(1));
    let shared = {
        let state = pool.state.lock().expect("pool state");
        Arc::downgrade(&state.entries[0].shared)
    };
    drop(pool);
    tokio::time::timeout(Duration::from_secs(1), async {
        while shared.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("aborted worker releases its inventory");
}

#[cfg(unix)]
fn fixture_spec(mode: &str, directory: &tempfile::TempDir) -> Arc<AgentSpec> {
    let mut spec = AgentSpec::acp(
        "pool-fixture",
        crate::agent::Command::new(
            "sh",
            [
                "-c".to_owned(),
                include_str!("../assets/fixture.sh").to_owned(),
            ],
        ),
    );
    spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
        directory.path().join("pid").to_string_lossy().into_owned(),
    );
    Arc::new(spec)
}

#[cfg(unix)]
async fn wait_for_idle(pool: &Pool, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ready = {
                let state = pool.state.lock().expect("pool state");
                state.entries[0]
                    .shared
                    .state
                    .lock()
                    .expect("inventory")
                    .idle
                    .len()
                    == count
            };
            if ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("pool reaches idle target");
}

#[cfg(unix)]
#[tokio::test]
async fn invalidation_discards_credentials_without_disabling_future_prewarming() {
    let directory = tempfile::tempdir().expect("directory");
    let spec = fixture_spec("load", &directory);
    let pool = Pool::default();
    pool.maintain(spec.clone(), 1, Duration::from_secs(5));
    wait_for_idle(&pool, 1).await;
    let original = std::fs::read_to_string(directory.path().join("pid")).expect("original pid");
    pool.invalidate();
    assert!(pool.take(&spec).is_none());
    pool.maintain(spec.clone(), 1, Duration::from_secs(5));
    wait_for_idle(&pool, 1).await;
    let replacement =
        std::fs::read_to_string(directory.path().join("pid")).expect("replacement pid");
    assert_ne!(original, replacement);
    let warm = pool.take(&spec).expect("fresh credential state");
    warm.shutdown().await;
    pool.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn checkout_refills_without_creating_a_session_and_matches_the_entire_spec() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture_spec("load", &directory);
    let pool = Pool::default();
    pool.maintain(spec.clone(), 1, Duration::from_secs(5));
    wait_for_idle(&pool, 1).await;
    let mut different = (*spec).clone();
    different.env.insert("EXTRA".into(), "override".into());
    assert!(pool.take(&Arc::new(different)).is_none());

    let first = pool
        .take(&Arc::new((*spec).clone()))
        .expect("warm checkout");
    assert!(first.is_alive());
    wait_for_idle(&pool, 1).await;
    assert!(!directory.path().join("pid.new").exists());
    assert!(!directory.path().join("pid.load").exists());

    pool.maintain(spec.clone(), 0, Duration::from_secs(5));
    assert!(pool.take(&spec).is_none());
    pool.shutdown().await;
    assert!(
        first.is_alive(),
        "checkout transfers ownership out of the pool"
    );
    first.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_cancels_and_joins_a_hung_initialization() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let pool = Pool::default();
    pool.maintain(
        fixture_spec("hang_start", &directory),
        1,
        Duration::from_secs(60),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.path().join("pid").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("initialization starts");
    tokio::time::timeout(Duration::from_secs(5), pool.shutdown())
        .await
        .expect("shutdown does not wait for the initialization timeout");
    assert!(pool.state.lock().expect("pool state").entries.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn timed_out_initialization_is_retried_in_the_background() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = fixture_spec("hang_start", &directory);
    let pool = Pool::default();
    pool.maintain(spec.clone(), 1, Duration::from_millis(100));
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut first = None;
        loop {
            if let Ok(pid) = std::fs::read_to_string(directory.path().join("pid"))
                && !pid.trim().is_empty()
            {
                if let Some(first) = &first {
                    if first != &pid {
                        break;
                    }
                } else {
                    first = Some(pid);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a timeout releases the slot for a retry");
    assert!(pool.take(&spec).is_none());
    pool.shutdown().await;
}
