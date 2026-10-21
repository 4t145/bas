use super::*;
use bas::{acp, model::TurnId};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Observed {
    launches: AtomicUsize,
    prompts: AtomicUsize,
    restored_ids: Mutex<Vec<String>>,
    targets: Mutex<BTreeMap<AgentId, usize>>,
}

struct Driver(Arc<Observed>);

struct Backend {
    id: String,
    observed: Arc<Observed>,
    _updates: mpsc::Sender<agent::Update>,
}

#[async_trait::async_trait]
impl agent::Agent for Backend {
    fn acp_session_id(&self) -> Option<String> {
        Some(self.id.clone())
    }

    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        self.observed.prompts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }

    async fn shutdown(&mut self) {}
}

#[async_trait::async_trait]
impl agent::Driver for Driver {
    fn kind(&self) -> Kind {
        Kind::Acp
    }

    fn maintain_pool(&self, spec: Arc<AgentSpec>, target: usize, _: Duration) {
        self.0
            .targets
            .lock()
            .expect("targets")
            .insert(spec.id.clone(), target);
    }

    async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn agent::Agent>, agent::Error> {
        self.0.launches.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Backend {
            id: format!("raw-{}", request.session),
            observed: Arc::clone(&self.0),
            _updates: request.updates,
        }))
    }

    async fn restore(
        &self,
        request: LaunchRequest,
        id: String,
    ) -> Result<Box<dyn agent::Agent>, agent::Error> {
        self.0
            .restored_ids
            .lock()
            .expect("restores")
            .push(id.clone());
        Ok(Box::new(Backend {
            id,
            observed: Arc::clone(&self.0),
            _updates: request.updates,
        }))
    }
}

fn catalog(ids: &[&str], observed: &Arc<Observed>) -> Arc<Catalog> {
    let mut catalog = Catalog::new();
    for id in ids {
        catalog
            .insert(
                AgentSpec::acp(*id, Command::new("unused", [])),
                Arc::new(Driver(Arc::clone(observed))),
            )
            .expect("register");
    }
    Arc::new(catalog)
}

async fn create(registry: &Registry, agent: &str) -> Arc<Session> {
    registry
        .create(CreateRequest {
            agent: agent.into(),
            owner: Owner::current(),
            cwd: None,
            title: None,
        })
        .await
        .expect("create")
}

#[tokio::test]
async fn cancelling_a_ready_but_unpublished_create_does_not_select_it() {
    let observed = Arc::new(Observed::default());
    let storage = Arc::new(storage::Memory::new(Limits::default().history_capacity));
    let registry = Arc::new(Registry::with_storage(
        catalog(&["a", "b", "c"], &observed),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    ));
    let c = create(&registry, "c").await;
    create(&registry, "a").await;
    assert_eq!(
        storage.last_used_agent().await.expect("selection"),
        Some("a".into())
    );
    let registration = registry.sessions.write().await;
    let creating = {
        let registry = registry.clone();
        tokio::spawn(async move { create(&registry, "b").await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while !storage
            .list()
            .await
            .expect("stored sessions")
            .iter()
            .any(|meta| meta.agent.as_str() == "b" && meta.status == SessionStatus::Idle)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("ready commit before blocked registry publication");
    assert_eq!(
        storage.last_used_agent().await.expect("still a"),
        Some("a".into())
    );
    c.prompt(vec!["a newer legitimate use".into()])
        .await
        .expect("accepted");
    creating.abort();
    assert!(matches!(creating.await, Err(error) if error.is_cancelled()));
    drop(registration);
    tokio::time::timeout(Duration::from_secs(5), async {
        while storage
            .list()
            .await
            .expect("sessions")
            .iter()
            .any(|meta| meta.agent.as_str() == "b")
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("unpublished row removed");
    assert_eq!(
        storage.last_used_agent().await.expect("newer c preserved"),
        Some("c".into())
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn restart_prewarms_committed_selection_without_resurrecting_sessions_or_prompts() {
    let directory = tempfile::tempdir().expect("private database directory");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private database directory permissions");
    let path = directory.path().join("sessions.sqlite3");
    let original = Arc::new(Observed::default());
    let (id, backend_id) = {
        let storage = Arc::new(storage::LocalDb::open(&path).await.expect("open"));
        let registry = Registry::with_storage(
            catalog(&["a", "b"], &original),
            Arc::new(LocalHost::default()),
            Limits::default(),
            storage.clone(),
        );
        let a = create(&registry, "a").await;
        let b = create(&registry, "b").await;
        a.prompt(vec!["accepted even if its caller disconnects".into()])
            .await
            .expect("prompt");
        // A late response and unrelated metadata must not override commit order.
        registry.record_agent_use("b".into());
        b.set_title("newer metadata, not newer usage")
            .await
            .expect("rename");
        assert_eq!(
            storage.last_used_agent().await.expect("selection"),
            Some("a".into())
        );
        let id = a.id();
        let backend_id = a.backend_id.get().expect("backend id").clone();
        registry.shutdown().await;
        (id, backend_id)
    };
    assert_eq!(original.prompts.load(Ordering::SeqCst), 1);

    let observed = Arc::new(Observed::default());
    let storage = Arc::new(storage::LocalDb::open(&path).await.expect("reopen"));
    let mut registry = Registry::with_storage(
        catalog(&["a", "b"], &observed),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    );
    registry.recover().await.expect("recover");
    assert_eq!(registry.list().await.len(), 2);
    assert_eq!(
        registry.get(id).await.expect("session").meta().status,
        SessionStatus::Idle
    );
    assert_eq!(observed.launches.load(Ordering::SeqCst), 0);
    assert!(observed.restored_ids.lock().expect("restores").is_empty());
    assert_eq!(
        observed.prompts.load(Ordering::SeqCst),
        0,
        "never resend an interrupted prompt"
    );
    registry.maintain_backends().await;
    assert_eq!(
        *observed.targets.lock().expect("targets"),
        BTreeMap::from([("a".into(), 1), ("b".into(), 0),])
    );
    assert!(
        observed.restored_ids.lock().expect("restores").is_empty(),
        "pool policy is not session recovery"
    );

    registry
        .set_acp_policy(AcpLifecycle {
            default_agent: Some("b".into()),
            ..Default::default()
        })
        .expect("default target");
    registry.maintain_backends().await;
    assert_eq!(
        *observed.targets.lock().expect("targets"),
        BTreeMap::from([("a".into(), 0), ("b".into(), 1),])
    );
    registry
        .set_acp_policy(AcpLifecycle {
            default_agent: Some("b".into()),
            pools: BTreeMap::from([("a".into(), 2), ("b".into(), 0)]),
            ..Default::default()
        })
        .expect("explicit targets");
    registry.maintain_backends().await;
    assert_eq!(
        *observed.targets.lock().expect("targets"),
        BTreeMap::from([("a".into(), 2), ("b".into(), 0),])
    );
    assert_eq!(
        storage
            .last_used_agent()
            .await
            .expect("unchanged preference"),
        Some("a".into())
    );

    let session = registry.get(id).await.expect("session");
    let lease = registry.use_backend(&session).await.expect("lazy restore");
    assert_eq!(
        *observed.restored_ids.lock().expect("restores"),
        [backend_id]
    );
    drop(lease);
    assert_eq!(observed.prompts.load(Ordering::SeqCst), 0);

    registry
        .set_acp_policy(AcpLifecycle::default())
        .expect("automatic target");
    registry.replace_catalog(catalog(&["b"], &observed));
    observed.targets.lock().expect("targets").clear();
    registry.maintain_backends().await;
    assert_eq!(
        *observed.targets.lock().expect("targets"),
        BTreeMap::from([("b".into(), 0)]),
        "removed last-used agent must not launch or choose an arbitrary replacement"
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn preference_write_failure_does_not_revoke_a_published_creation() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("private state");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private permissions");
    let path = directory.path().join("sessions.sqlite3");
    let storage = Arc::new(storage::LocalDb::open(&path).await.expect("open"));
    let writer = rusqlite::Connection::open(&path).expect("external writer");
    writer
        .execute_batch(
            "CREATE TRIGGER refuse_selection BEFORE INSERT ON agent_selection
         BEGIN SELECT RAISE(ABORT, 'injected selection failure'); END;",
        )
        .expect("inject preference-only failure");
    let registry = Registry::with_storage(
        catalog(&["a"], &Arc::new(Observed::default())),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    );
    let session = create(&registry, "a").await;
    assert!(registry.get(session.id()).await.is_ok());
    assert!(storage.load(session.id()).await.expect("load").is_some());
    assert_eq!(session.meta().status, SessionStatus::Idle);
    assert_eq!(
        storage
            .last_used_agent()
            .await
            .expect("unchanged selection"),
        None
    );
    registry.shutdown().await;
}
