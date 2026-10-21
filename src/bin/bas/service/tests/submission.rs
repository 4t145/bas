use super::*;
use crate::{
    agent::{self, Agent, AgentSpec, Driver, Kind, LaunchRequest, Update},
    storage::{self, Memory, PromptAcceptance, ResumeData, Storage, StoredSession},
};
use async_trait::async_trait;
use bas::{
    acp,
    model::{
        Event, EventKind, PromptSubmission, PromptSubmissionOutcome, SessionId, SessionMeta, TurnId,
    },
    service::Client,
};
use futures::future::BoxFuture;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, mpsc};

#[derive(Default)]
struct Observations {
    launches: AtomicUsize,
    restores: AtomicUsize,
    dispatches: AtomicUsize,
    fail_dispatch: AtomicBool,
}

struct ObservedAgent {
    observations: Arc<Observations>,
    _updates: mpsc::Sender<Update>,
}

#[async_trait]
impl Agent for ObservedAgent {
    fn prompt(&mut self, _: TurnId, _: acp::PromptRequest) -> Result<(), agent::Error> {
        self.observations.dispatches.fetch_add(1, Ordering::SeqCst);
        if self.observations.fail_dispatch.load(Ordering::SeqCst) {
            return Err(agent::Error::Unsupported {
                agent: "embedded".into(),
                operation: "prompt",
            });
        }
        Ok(())
    }
    fn cancel(&mut self, _: acp::CancelNotification) -> Result<(), agent::Error> {
        Ok(())
    }
    async fn shutdown(&mut self) {}
}

struct ObservedDriver(Arc<Observations>);

#[async_trait]
impl Driver for ObservedDriver {
    fn kind(&self) -> Kind {
        Kind::Embedded
    }
    async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn Agent>, agent::Error> {
        self.0.launches.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(ObservedAgent {
            observations: self.0.clone(),
            _updates: request.updates,
        }))
    }
    async fn restore(
        &self,
        request: LaunchRequest,
        _: String,
    ) -> Result<Box<dyn Agent>, agent::Error> {
        self.0.restores.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(ObservedAgent {
            observations: self.0.clone(),
            _updates: request.updates,
        }))
    }
}

/// Pauses after acceptance is durable, before the commit caller can dispatch.
struct ControlledStore {
    inner: Memory,
    pause: AtomicBool,
    accepted: Notify,
    release: Notify,
    fail_interruption: AtomicBool,
    fail_load: AtomicBool,
    loads: AtomicUsize,
}

impl ControlledStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Memory::new(NonZeroUsize::new(100).expect("capacity")),
            pause: AtomicBool::new(false),
            accepted: Notify::new(),
            release: Notify::new(),
            fail_interruption: AtomicBool::new(false),
            fail_load: AtomicBool::new(false),
            loads: AtomicUsize::new(0),
        })
    }
}

impl Storage for ControlledStore {
    fn create(
        &self,
        meta: SessionMeta,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), storage::Error>> {
        self.inner.create(meta, resume)
    }
    fn commit<'a>(
        &'a self,
        meta: SessionMeta,
        events: &'a [Event],
        acceptance: Option<PromptAcceptance>,
    ) -> BoxFuture<'a, Result<(), storage::Error>> {
        Box::pin(async move {
            if self.fail_interruption.load(Ordering::SeqCst)
                && events
                    .iter()
                    .any(|event| matches!(event.kind, EventKind::TurnInterrupted { .. }))
            {
                return Err(storage::Error::InvalidCommit("interruption failure"));
            }
            self.inner.commit(meta, events, acceptance).await?;
            if acceptance.is_some() && self.pause.swap(false, Ordering::SeqCst) {
                self.accepted.notify_one();
                self.release.notified().await;
            }
            Ok(())
        })
    }
    fn acceptance(
        &self,
        session: SessionId,
        turn: TurnId,
    ) -> BoxFuture<'_, Result<Option<PromptAcceptance>, storage::Error>> {
        self.inner.acceptance(session, turn)
    }
    fn save_resume(
        &self,
        session: SessionId,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), storage::Error>> {
        self.inner.save_resume(session, resume)
    }
    fn load(
        &self,
        session: SessionId,
    ) -> BoxFuture<'_, Result<Option<StoredSession>, storage::Error>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        if self.fail_load.load(Ordering::SeqCst) {
            return Box::pin(async { Err(storage::Error::WorkerClosed) });
        }
        self.inner.load(session)
    }
    fn list(&self) -> BoxFuture<'_, Result<Vec<SessionMeta>, storage::Error>> {
        self.inner.list()
    }
    fn page(
        &self,
        session: SessionId,
        after: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<storage::Page, storage::Error>> {
        self.inner.page(session, after, limit)
    }
    fn window(
        &self,
        session: SessionId,
        before: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<storage::Window, storage::Error>> {
        self.inner.window(session, before, limit)
    }
    fn tail(
        &self,
        session: SessionId,
        count: usize,
    ) -> BoxFuture<'_, Result<Vec<Event>, storage::Error>> {
        self.inner.tail(session, count)
    }
    fn delete(&self, session: SessionId) -> BoxFuture<'_, Result<bool, storage::Error>> {
        self.inner.delete(session)
    }
    fn is_persistent(&self) -> bool {
        false
    }
}

fn registry_with(storage: Arc<dyn Storage>, observations: Arc<Observations>) -> Registry {
    let mut catalog = Catalog::new();
    catalog
        .insert(
            AgentSpec::embedded(),
            Arc::new(ObservedDriver(observations)),
        )
        .expect("driver");
    Registry::with_storage(
        Arc::new(catalog),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage,
    )
}

struct Server {
    task: tokio::task::JoinHandle<()>,
    client: Client,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(registry: Arc<Registry>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(Service::new(registry).into_server())
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .expect("server");
    });
    let client = Client::connect(format!("http://{address}"))
        .await
        .expect("client");
    Server { task, client }
}

fn submission(session: SessionId) -> PromptSubmission {
    PromptSubmission::new(acp::PromptRequest::new(
        session.to_string(),
        vec!["hello".into()],
    ))
}

#[tokio::test]
async fn real_grpc_lost_response_and_cancelled_call_recover_one_acceptance() {
    for cancel in [false, true] {
        let storage = ControlledStore::new();
        let observations = Arc::new(Observations::default());
        let registry = Arc::new(registry_with(storage.clone(), observations.clone()));
        let session = create(&registry).await;
        let mut server = serve(registry.clone()).await;
        let request = submission(session.id());
        let before = session.snapshot();
        assert_eq!(
            server
                .client
                .prompt_acceptance(session.id(), request.turn)
                .await
                .expect("unknown lookup"),
            None
        );
        assert_eq!(session.snapshot(), before);
        assert_eq!(observations.dispatches.load(Ordering::SeqCst), 0);
        if cancel {
            storage.pause.store(true, Ordering::SeqCst);
            let mut client = server.client.clone();
            let request = request.clone();
            let call = tokio::spawn(async move { client.submit_prompt(&request).await });
            tokio::time::timeout(Duration::from_secs(5), storage.accepted.notified())
                .await
                .expect("durable acceptance reached");
            assert_eq!(observations.dispatches.load(Ordering::SeqCst), 0);
            call.abort();
            assert!(call.await.expect_err("cancelled caller").is_cancelled());
            storage.release.notify_one();
        } else {
            // The application deliberately discards the successful transport response.
            let _ = server
                .client
                .prompt(pb::PromptSubmission::from(&request))
                .await
                .expect("transport");
        }
        let receipt = tokio::time::timeout(
            Duration::from_secs(5),
            server.client.prompt_acceptance(session.id(), request.turn),
        )
        .await
        .expect("lookup deadline")
        .expect("lookup")
        .expect("accepted");
        assert_eq!(receipt.turn, request.turn);
        assert_eq!(receipt.cursor, before.meta.revision.next());
        let history = storage
            .page(session.id(), None, 100)
            .await
            .expect("history");
        assert_eq!(
            history
                .events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::TurnStarted { .. }))
                .count(),
            1
        );
        assert_eq!(
            server.client.submit_prompt(&request).await.expect("retry"),
            PromptSubmissionOutcome::Accepted(receipt)
        );
        let mut conflict = request.clone();
        conflict.request.prompt = vec!["changed".into()];
        assert!(
            matches!(server.client.submit_prompt(&conflict).await.expect("explicit rejection"),
            PromptSubmissionOutcome::Rejected(rejection) if rejection.kind == bas::model::PromptRejectionKind::Conflict)
        );
        assert_eq!(
            storage
                .page(session.id(), None, 100)
                .await
                .expect("history"),
            history
        );
        assert_eq!(observations.dispatches.load(Ordering::SeqCst), 1);
        registry.shutdown().await;
    }
}

#[tokio::test]
async fn healthy_busy_archived_and_inactive_sessions_explicitly_reject_new_submissions() {
    use bas::model::PromptRejectionKind;
    let storage = ControlledStore::new();
    let observations = Arc::new(Observations::default());
    let registry = Arc::new(registry_with(storage.clone(), observations.clone()));
    let session = create(&registry).await;
    let mut server = serve(registry.clone()).await;
    let original = submission(session.id());
    let accepted = server
        .client
        .submit_prompt(&original)
        .await
        .expect("accepted");
    for kind in [
        PromptRejectionKind::Busy,
        PromptRejectionKind::Archived,
        PromptRejectionKind::Unavailable,
    ] {
        if kind == PromptRejectionKind::Archived {
            session.try_set_archived(true).await.expect("archive");
        } else if kind == PromptRejectionKind::Unavailable {
            session.try_set_archived(false).await.expect("unarchive");
            session.shutdown().await;
        }
        let request = submission(session.id());
        let before = session.snapshot();
        let history = storage
            .page(session.id(), None, 100)
            .await
            .expect("history");
        assert!(
            matches!(server.client.submit_prompt(&request).await.expect("explicit rejection"),
            PromptSubmissionOutcome::Rejected(rejection) if rejection.kind == kind)
        );
        assert_eq!(
            server
                .client
                .prompt_acceptance(session.id(), request.turn)
                .await
                .expect("lookup"),
            None
        );
        assert_eq!(
            server
                .client
                .submit_prompt(&original)
                .await
                .expect("existing receipt wins"),
            accepted
        );
        assert_eq!(session.snapshot(), before);
        assert_eq!(
            storage
                .page(session.id(), None, 100)
                .await
                .expect("history"),
            history
        );
        assert_eq!(observations.dispatches.load(Ordering::SeqCst), 1);
    }
    registry.shutdown().await;
}
