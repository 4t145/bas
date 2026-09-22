use std::{collections::BTreeMap, num::NonZeroUsize, path::PathBuf, sync::Arc, time::Duration};

use basr::model::{AgentId, Owner, SessionId, SessionMeta};
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use super::{Completion, Session, SessionError};
use crate::agent::{self, Catalog, Host, LaunchRequest};

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
    #[error("cannot resolve the session working directory: {0}")]
    Cwd(#[source] std::io::Error),
    #[error(transparent)]
    Agent(#[from] agent::Error),
    #[error(transparent)]
    Session(#[from] SessionError),
}

#[derive(Debug, thiserror::Error)]
#[error("session {0} not found")]
pub struct NotFound(pub SessionId);

pub struct Registry {
    catalog: std::sync::RwLock<Arc<Catalog>>,
    host: Arc<dyn Host>,
    limits: Limits,
    sessions: RwLock<BTreeMap<SessionId, Arc<Session>>>,
    stopping: CancellationToken,
    completions: Option<mpsc::Sender<Completion>>,
}

impl Registry {
    pub fn new(catalog: Arc<Catalog>, host: Arc<dyn Host>, limits: Limits) -> Self {
        Self {
            catalog: std::sync::RwLock::new(catalog),
            host,
            limits,
            sessions: RwLock::new(BTreeMap::new()),
            stopping: CancellationToken::new(),
            completions: None,
        }
    }

    pub fn with_completions(mut self, completions: mpsc::Sender<Completion>) -> Self {
        self.completions = Some(completions);
        self
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        Arc::clone(&self.catalog.read().expect("catalog lock poisoned"))
    }

    /// Publishing a catalog never mutates or stops existing sessions.
    pub fn replace_catalog(&self, catalog: Arc<Catalog>) {
        *self.catalog.write().expect("catalog lock poisoned") = catalog;
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
            result = tokio::time::timeout(self.limits.startup_timeout, self.launch(request)) => {
                result.map_err(|_| CreateError::Timeout)?
            }
        }
    }

    async fn launch(&self, request: CreateRequest) -> Result<Arc<Session>, CreateError> {
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
        let cwd = tokio::fs::canonicalize(cwd)
            .await
            .map_err(CreateError::Cwd)?;
        basr::service::utf8_cwd(&cwd).map_err(|error| {
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
        let session = Session::new(meta, self.limits, self.completions.clone());
        let (updates, inbox) = mpsc::channel(self.limits.live_capacity.get());
        // Drain initialization updates too, so protocol startup cannot deadlock on a full queue.
        session.listen(inbox);
        let agent = catalog
            .launch(LaunchRequest {
                session: id,
                spec,
                cwd,
                host: Arc::clone(&self.host),
                updates,
            })
            .await?;
        session.ready(agent)?;
        let mut sessions = self.sessions.write().await;
        if self.stopping.is_cancelled() {
            return Err(CreateError::Stopping);
        }
        sessions.insert(session.id(), Arc::clone(&session));
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

    /// # Errors
    /// Returns NotFound for unknown or already removed sessions.
    pub async fn remove(&self, id: SessionId) -> Result<(), NotFound> {
        let session = self
            .sessions
            .write()
            .await
            .remove(&id)
            .ok_or(NotFound(id))?;
        session.shutdown().await;
        Ok(())
    }

    /// Prevents new sessions, cancels pending launches and joins every registered backend.
    pub async fn shutdown(&self) {
        self.stopping.cancel();
        let sessions = std::mem::take(&mut *self.sessions.write().await);
        futures::future::join_all(sessions.values().map(|session| session.shutdown())).await;
    }
}
