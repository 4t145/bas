use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Weak},
};

use tokio::sync::{RwLock, broadcast, mpsc};

use crate::{
    agent::{self, Catalog, Host, LaunchRequest, Update},
    model::{AgentId, Owner, SessionId, SessionMeta, SessionStatus},
};

use super::{Session, history::DEFAULT_CAPACITY};

/// Number of session-creation notifications a daemon-side watcher may lag.
const CREATED_BUFFER: usize = 64;

/// Sizes applied to every session the registry creates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Replayable events kept per session.
    pub history_capacity: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            history_capacity: DEFAULT_CAPACITY,
        }
    }
}

/// How to create a session.
pub struct CreateRequest {
    /// Configured backend to launch.
    pub agent: AgentId,
    /// User and machine asking for the session.
    pub owner: Owner,
    /// Working directory for the agent.
    pub cwd: Option<PathBuf>,
    /// Optional initial title.
    pub title: Option<String>,
}

/// Failure raised while creating a session.
#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("agent {0} is not configured")]
    UnknownAgent(AgentId),
    #[error(transparent)]
    Agent(#[from] agent::Error),
}

/// Failure raised while removing a session.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("session {0} not found")]
    NotFound(SessionId),
    #[error(transparent)]
    Agent(#[from] agent::Error),
}

/// Failure raised by a session lookup.
#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("session {0} not found")]
    NotFound(SessionId),
}

/// Owns every live session of the daemon.
pub struct Registry {
    catalog: Arc<Catalog>,
    host: Arc<dyn Host>,
    limits: Limits,
    sessions: RwLock<BTreeMap<SessionId, Arc<Session>>>,
    created: broadcast::Sender<Arc<Session>>,
}

impl Registry {
    /// Builds a registry that launches backends from `catalog`.
    #[must_use]
    pub fn new(catalog: Arc<Catalog>, host: Arc<dyn Host>, limits: Limits) -> Self {
        let (created, _) = broadcast::channel(CREATED_BUFFER);

        Self {
            catalog,
            host,
            limits,
            sessions: RwLock::new(BTreeMap::new()),
            created,
        }
    }

    /// Subscribes to sessions as they are created.
    ///
    /// Daemon-side watchers (notifications, audit logs) use this instead of
    /// polling [`Self::list`].
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Session>> {
        self.created.subscribe()
    }

    /// The configured backends.
    #[must_use]
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    /// Starts a backend and registers the resulting session.
    ///
    /// # Errors
    ///
    /// Returns [`CreateError::UnknownAgent`] when the request names a backend
    /// that is not configured, or [`CreateError::Agent`] when the backend
    /// cannot be started.
    pub async fn create(&self, request: CreateRequest) -> Result<Arc<Session>, CreateError> {
        let spec = self
            .catalog
            .spec(&request.agent)
            .cloned()
            .ok_or_else(|| CreateError::UnknownAgent(request.agent.clone()))?;

        let id = SessionId::new();
        let mut meta = SessionMeta::new(id, request.agent, request.owner.clone());
        meta.cwd = request.cwd.clone();
        meta.title = request.title.clone();

        let (updates, inbox) = mpsc::unbounded_channel();
        let agent = self
            .catalog
            .launch(LaunchRequest {
                session: id,
                spec,
                owner: request.owner,
                cwd: request.cwd,
                title: request.title,
                host: Arc::clone(&self.host),
                updates,
            })
            .await?;

        let session = Arc::new(Session::new(meta, agent, self.limits.history_capacity));
        tokio::spawn(publish_updates(Arc::downgrade(&session), inbox));
        session.set_status(SessionStatus::Idle);

        self.sessions.write().await.insert(id, Arc::clone(&session));
        let _ = self.created.send(Arc::clone(&session));
        Ok(session)
    }

    /// Looks up a live session.
    ///
    /// # Errors
    ///
    /// Returns [`LookupError::NotFound`] when the id is unknown.
    pub async fn get(&self, id: SessionId) -> Result<Arc<Session>, LookupError> {
        self.sessions
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(LookupError::NotFound(id))
    }

    /// Metadata of every live session, oldest first.
    pub async fn list(&self) -> Vec<SessionMeta> {
        let sessions: Vec<Arc<Session>> = self.sessions.read().await.values().cloned().collect();
        let mut metas: Vec<SessionMeta> = sessions.iter().map(|session| session.meta()).collect();
        metas.sort_by_key(|meta| (meta.created_at, meta.id));
        metas
    }

    /// Stops a session's backend and forgets it.
    ///
    /// # Errors
    ///
    /// Returns [`RemoveError::NotFound`] when the id is unknown, or
    /// [`RemoveError::Agent`] when the backend refuses to stop.
    pub async fn remove(&self, id: SessionId) -> Result<(), RemoveError> {
        let session = self
            .sessions
            .write()
            .await
            .remove(&id)
            .ok_or(RemoveError::NotFound(id))?;

        session.shutdown().await?;
        Ok(())
    }

    /// Number of live sessions.
    pub async fn len(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// Returns whether no session is live.
    pub async fn is_empty(&self) -> bool {
        self.sessions.read().await.is_empty()
    }
}

/// Forwards backend updates to the session while it is alive.
///
/// The task holds only a [`Weak`] handle so that dropping the registry's last
/// reference lets the session, its backend, and this task all end together.
async fn publish_updates(session: Weak<Session>, mut inbox: mpsc::UnboundedReceiver<Update>) {
    while let Some(update) = inbox.recv().await {
        let Some(session) = session.upgrade() else {
            return;
        };
        session.apply(update);
    }

    if let Some(session) = session.upgrade()
        && session.meta().status != SessionStatus::Stopped
    {
        session.fail();
    }
}
