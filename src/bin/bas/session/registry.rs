use std::{collections::BTreeMap, num::NonZeroUsize, path::PathBuf, sync::Arc, time::Duration};

use bas::{
    config::AgentRegistration,
    model::{AgentId, Owner, SessionId, SessionMeta, SessionStatus},
};
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use super::{Completion, Session, SessionError};
use crate::agent::{self, Catalog, Host, Kind, LaunchRequest};
use crate::storage::{self, ResumeData, Storage};

mod history;

pub const DEFAULT_HISTORY: usize = 4096;
pub const DEFAULT_BUFFER: usize = 1024;
pub const DEFAULT_STARTUP_TIMEOUT: u64 = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub history_capacity: NonZeroUsize,
    /// Both the backend update queue and each session's live broadcast are bounded.
    pub live_capacity: NonZeroUsize,
    pub startup_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            history_capacity: NonZeroUsize::new(DEFAULT_HISTORY)
                .expect("default history is nonzero"),
            live_capacity: NonZeroUsize::new(DEFAULT_BUFFER).expect("default buffer is nonzero"),
            startup_timeout: Duration::from_secs(DEFAULT_STARTUP_TIMEOUT),
        }
    }
}

pub struct CreateRequest {
    pub agent: AgentId,
    pub owner: Owner,
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("agent {0} is not configured")]
    UnknownAgent(AgentId),
    #[error("the daemon is stopping")]
    Stopping,
    #[error("backend startup timed out")]
    Timeout,
    #[error("session cannot be restored: {0}")]
    NotResumable(&'static str),
    #[error("cannot resolve the session working directory: {0}")]
    Cwd(#[source] std::io::Error),
    #[error(transparent)]
    Agent(#[from] agent::Error),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Storage(#[from] storage::Error),
}

#[derive(Debug, thiserror::Error)]
#[error("session {0} not found")]
pub struct NotFound(pub SessionId);

pub struct Registry {
    catalog: std::sync::RwLock<Arc<Catalog>>,
    host: Arc<dyn Host>,
    permissions: Arc<agent::permissions::Permissions>,
    elicitations: Arc<agent::elicitation::Elicitations>,
    limits: Limits,
    sessions: Arc<RwLock<BTreeMap<SessionId, Arc<Session>>>>,
    stopping: CancellationToken,
    completions: Option<mpsc::Sender<Completion>>,
    storage: Option<Arc<dyn Storage>>,
    history_gate: tokio::sync::Mutex<()>,
}

/// A failed or cancelled launch has never returned its id to a client.
struct PendingSession {
    id: SessionId,
    storage: Option<Arc<dyn Storage>>,
    registered: bool,
}

impl PendingSession {
    /// # Errors
    /// Returns storage creation failures without acquiring permission to delete the record.
    async fn create(
        meta: SessionMeta,
        storage: Option<Arc<dyn Storage>>,
    ) -> Result<Self, storage::Error> {
        // The write and its compensation share an owner even after startup is cancelled.
        // Dropping the detached task's unclaimed result drops the successful-create guard.
        tokio::spawn(async move {
            if let Some(storage) = &storage {
                storage.create(meta.clone(), None).await?;
            }
            Ok(Self {
                id: meta.id,
                storage,
                registered: false,
            })
        })
        .await
        .expect("session creation task panicked")
    }
}

impl Drop for PendingSession {
    fn drop(&mut self) {
        if !self.registered
            && let Some(storage) = self.storage.take()
        {
            let id = self.id;
            tokio::spawn(async move {
                if let Err(error) = storage.delete(id).await {
                    tracing::warn!(session = %id, %error, "could not discard an unregistered session");
                }
            });
        }
    }
}

impl Registry {
    pub fn new(catalog: Arc<Catalog>, host: Arc<dyn Host>, limits: Limits) -> Self {
        let permissions = Arc::new(agent::permissions::Permissions::default());
        let elicitations = Arc::new(agent::elicitation::Elicitations::default());
        let host = elicitations.host(permissions.host(host));
        Self {
            catalog: std::sync::RwLock::new(catalog),
            host,
            permissions,
            elicitations,
            limits,
            sessions: Arc::new(RwLock::new(BTreeMap::new())),
            stopping: CancellationToken::new(),
            completions: None,
            storage: None,
            history_gate: tokio::sync::Mutex::new(()),
        }
    }

    pub fn with_storage(
        catalog: Arc<Catalog>,
        host: Arc<dyn Host>,
        limits: Limits,
        storage: Arc<dyn Storage>,
    ) -> Self {
        let mut registry = Self::new(catalog, host, limits);
        registry.storage = Some(storage);
        registry
    }

    /// Rebuild metadata without launching backends.
    pub async fn recover(&mut self) -> Result<(), storage::Error> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        let sessions = Arc::get_mut(&mut self.sessions)
            .expect("registry is uniquely owned during recovery")
            .get_mut();
        for meta in storage.list().await? {
            let running = matches!(meta.status, SessionStatus::Idle | SessionStatus::Working);
            let session = Session::with_storage(
                meta,
                self.limits,
                self.completions.clone(),
                Some(Arc::clone(storage)),
                !running,
            );
            if let Some(stored) = storage.load(session.id()).await?
                && let Some(resume) = stored.resume
            {
                session.remember_backend_id(resume.acp_session_id()?.to_owned());
            }
            if let Err(error) = session.recover().await {
                tracing::error!(session = %session.id(), %error, "could not commit session recovery");
            }
            sessions.insert(session.id(), session);
        }
        Ok(())
    }

    pub fn with_completions(mut self, completions: mpsc::Sender<Completion>) -> Self {
        self.completions = Some(completions);
        self
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        Arc::clone(&self.catalog.read().expect("catalog lock poisoned"))
    }

    pub fn startup_timeout(&self) -> Duration {
        self.limits.startup_timeout
    }

    pub fn permissions(&self) -> &Arc<agent::permissions::Permissions> {
        &self.permissions
    }

    pub fn elicitations(&self) -> &Arc<agent::elicitation::Elicitations> {
        &self.elicitations
    }

    /// Publishing a catalog never mutates or stops existing sessions.
    #[cfg(test)]
    pub fn replace_catalog(&self, catalog: Arc<Catalog>) {
        let mut current = self.catalog.write().expect("catalog lock poisoned");
        if current.registrations().eq(catalog.registrations()) {
            return;
        }
        *current = catalog;
    }

    /// Read and publish while holding one generation lock, so a file watcher
    /// cannot publish an older read after a management operation's newer save.
    ///
    /// # Errors
    /// Preserves the current catalog on read or validation failure.
    pub fn reload_catalog(
        &self,
        load: impl FnOnce() -> anyhow::Result<Vec<AgentRegistration>>,
    ) -> anyhow::Result<Vec<AgentRegistration>> {
        let mut current = self.catalog.write().expect("catalog lock poisoned");
        let agents = load()?;
        let catalog = Catalog::from_registrations(agents.clone())?;
        if !current.registrations().eq(catalog.registrations()) {
            *current = Arc::new(catalog);
        }
        Ok(agents)
    }

    /// Lets connection-scoped streams end before the server finishes draining.
    pub async fn stopped(&self) {
        self.stopping.cancelled().await;
    }

    /// Initialization is bounded and cancelled when the request or daemon stops.
    ///
    /// # Errors
    /// Reports unknown backends, invalid directories, startup failures, timeout or shutdown.
    pub async fn create(&self, request: CreateRequest) -> Result<Arc<Session>, CreateError> {
        tokio::select! {
            biased;
            () = self.stopping.cancelled() => Err(CreateError::Stopping),
            result = tokio::time::timeout(self.limits.startup_timeout, self.launch(request, None)) => {
                result.map_err(|_| CreateError::Timeout)?
            }
        }
    }

    async fn launch(
        &self,
        request: CreateRequest,
        backend_session_id: Option<String>,
    ) -> Result<Arc<Session>, CreateError> {
        // Resolve both the specification and driver from one generation.
        let catalog = self.catalog();
        let spec = catalog
            .spec(&request.agent)
            .cloned()
            .ok_or_else(|| CreateError::UnknownAgent(request.agent.clone()))?;
        let cwd = match request.cwd {
            Some(cwd) => cwd,
            None => std::env::current_dir().map_err(CreateError::Cwd)?,
        };
        let cwd = if backend_session_id.is_some() {
            if !cwd.is_absolute() {
                return Err(CreateError::NotResumable(
                    "import requires an absolute Agent workspace",
                ));
            }
            cwd
        } else {
            tokio::fs::canonicalize(cwd)
                .await
                .map_err(CreateError::Cwd)?
        };
        bas::service::utf8_cwd(&cwd).map_err(|error| {
            CreateError::Cwd(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
        })?;
        if !tokio::fs::metadata(&cwd)
            .await
            .map_err(CreateError::Cwd)?
            .is_dir()
        {
            return Err(CreateError::Cwd(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "workspace must be a directory",
            )));
        }
        let id = SessionId::new();
        let mut meta = SessionMeta::new(id, request.agent, request.owner);
        meta.cwd = Some(cwd.clone());
        meta.title = request.title;
        let mut pending = PendingSession::create(meta.clone(), self.storage.clone()).await?;
        let session = Session::with_storage(
            meta,
            self.limits,
            self.completions.clone(),
            self.storage.clone(),
            true,
        );
        let (updates, inbox) = mpsc::channel(self.limits.live_capacity.get());
        // Drain initialization updates too, so protocol startup cannot deadlock on a full queue.
        session.listen(inbox);
        let launch = LaunchRequest {
            session: id,
            spec,
            cwd,
            host: Arc::clone(&self.host),
            updates: updates.clone(),
        };
        let mut agent = if let Some(backend_id) = backend_session_id {
            let agent = catalog.restore(launch, backend_id).await?;
            // An import has no existing transcript: retain the Agent's load replay.
            // Drain it before publishing the ready snapshot or returning the ID.
            let (ack, drained) = tokio::sync::oneshot::channel();
            updates
                .send(agent::Update::Barrier(ack))
                .await
                .map_err(|_| CreateError::NotResumable("import listener closed during replay"))?;
            drained
                .await
                .map_err(|_| CreateError::NotResumable("import listener closed during replay"))?;
            agent
        } else {
            catalog.launch(launch).await?
        };
        if let Some(storage) = &self.storage
            && let Err(error) = storage
                .save_resume(id, agent.acp_session_id().map(ResumeData::acp))
                .await
        {
            agent.shutdown().await;
            return Err(error.into());
        }
        session.ready(agent).await?;
        let mut sessions = self.sessions.write().await;
        if self.stopping.is_cancelled() {
            return Err(CreateError::Stopping);
        }
        sessions.insert(session.id(), Arc::clone(&session));
        pending.registered = true;
        Ok(session)
    }

    /// # Errors
    /// Returns NotFound for unknown or removed sessions.
    pub async fn get(&self, id: SessionId) -> Result<Arc<Session>, NotFound> {
        self.sessions
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(NotFound(id))
    }

    /// Restore the backend on demand without holding a registry or state lock across launch.
    pub async fn ensure_restored(&self, session: &Arc<Session>) -> Result<(), CreateError> {
        tokio::select! {
            biased;
            () = self.stopping.cancelled() => Err(CreateError::Stopping),
            result = tokio::time::timeout(self.limits.startup_timeout, self.restore(session)) => {
                result.map_err(|_| CreateError::Timeout)?
            }
        }
    }

    async fn restore(&self, session: &Arc<Session>) -> Result<(), CreateError> {
        session.check_health()?;
        if session.is_restored() {
            return Ok(());
        }
        let gate = Arc::clone(&session.restore_gate).lock_owned().await;
        session.check_health()?;
        if session.is_restored() {
            return Ok(());
        }
        let meta = session.meta();
        let catalog = self.catalog();
        let spec = catalog
            .spec(&meta.agent)
            .cloned()
            .ok_or_else(|| CreateError::UnknownAgent(meta.agent.clone()))?;
        let cwd = meta.cwd.clone().ok_or_else(|| {
            CreateError::Cwd(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "session workspace is missing",
            ))
        })?;
        let stored = self
            .storage
            .as_ref()
            .expect("recovered session has storage")
            .load(meta.id)
            .await?
            .ok_or(storage::Error::NotFound(meta.id))?;
        if spec.kind == Kind::Acp && stored.resume.is_none() {
            return Err(CreateError::NotResumable(
                "ACP session has no saved backend ID",
            ));
        }
        if spec.kind != Kind::Acp && stored.resume.is_some() {
            return Err(CreateError::NotResumable(
                "configured backend type differs from saved session",
            ));
        }
        let (updates, inbox) = mpsc::channel(self.limits.live_capacity.get());
        session.listen(inbox);
        let launch = LaunchRequest {
            session: meta.id,
            spec,
            cwd,
            host: Arc::clone(&self.host),
            updates: updates.clone(),
        };
        let backend_session_id = stored
            .resume
            .map(|resume| resume.acp_session_id().map(str::to_owned))
            .transpose()?
            .unwrap_or_default();
        let agent = catalog.restore(launch, backend_session_id).await?;
        session.check_health()?;
        // ACP requires all session/load replay notifications before its response.
        // The SDK dispatches notification callbacks serially in wire order; this
        // barrier drains their queued updates before normal publication resumes.
        // https://agentclientprotocol.com/protocol/session-setup#loading-sessions
        let (ack, drained) = tokio::sync::oneshot::channel();
        updates
            .send(agent::Update::Barrier(ack))
            .await
            .map_err(|_| {
                CreateError::NotResumable("backend update listener closed during restore")
            })?;
        drained.await.map_err(|_| {
            CreateError::NotResumable("backend update listener closed during restore")
        })?;
        session.restore_agent(agent, gate).await?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn list(&self) -> Vec<SessionMeta> {
        let mut metas: Vec<_> = self
            .sessions
            .read()
            .await
            .values()
            .map(|session| session.meta())
            .collect();
        metas.sort_by_key(|meta| (meta.created_at, meta.id));
        metas
    }

    pub async fn snapshots(&self) -> Vec<bas::model::SessionSnapshot> {
        let mut snapshots: Vec<_> = self
            .sessions
            .read()
            .await
            .values()
            .map(|session| session.snapshot())
            .collect();
        snapshots.sort_by_key(|snapshot| (snapshot.meta.created_at, snapshot.meta.id));
        snapshots
    }

    /// # Errors
    /// Returns NotFound for unknown or already removed sessions.
    pub async fn remove(&self, id: SessionId) -> Result<(), CreateError> {
        let session = self
            .sessions
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(storage::Error::NotFound(id))?;
        let sessions = Arc::clone(&self.sessions);
        let storage = self.storage.clone();
        let permissions = Arc::clone(&self.permissions);
        let elicitations = Arc::clone(&self.elicitations);
        // Deletion and registry eviction form one owned operation: an RPC
        // disconnect cannot leave an in-memory session pointing at a deleted row.
        tokio::spawn(async move {
            let _gate = session.restore_gate.lock().await;
            if storage
                .as_ref()
                .is_some_and(|storage| storage.is_persistent())
            {
                session.delete_storage().await?;
                sessions.write().await.remove(&id);
                permissions.close(id);
                elicitations.close(id);
                session.suspend().await;
            } else {
                sessions.write().await.remove(&id);
                permissions.close(id);
                elicitations.close(id);
                session.shutdown().await;
                if let Some(storage) = storage {
                    storage.delete(id).await?;
                }
            }
            Ok(())
        })
        .await
        .expect("session removal panicked")
    }

    /// Prevents new sessions, cancels pending launches and joins every registered backend.
    pub async fn shutdown(&self) {
        self.stopping.cancel();
        self.permissions.cancel_all();
        self.elicitations.cancel_all();
        let sessions = std::mem::take(&mut *self.sessions.write().await);
        if self
            .storage
            .as_ref()
            .is_some_and(|storage| storage.is_persistent())
        {
            futures::future::join_all(sessions.values().map(|session| async {
                let _gate = session.restore_gate.lock().await;
                session.suspend().await;
            }))
            .await;
        } else {
            futures::future::join_all(sessions.values().map(|session| async {
                let _gate = session.restore_gate.lock().await;
                session.shutdown().await;
            }))
            .await;
        }
    }
}
