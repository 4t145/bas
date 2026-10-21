use super::*;
use crate::session::BackendUse;

/// Idle expiry is best-effort within one scan interval; policy changes wake it immediately.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(5);

#[cfg(all(test, unix))]
mod tests;

/// Credentials can change even if an authentication RPC fails or is cancelled.
#[must_use]
pub struct CredentialsChange {
    registry: Arc<Registry>,
    agent: AgentId,
}

impl Drop for CredentialsChange {
    fn drop(&mut self) {
        self.registry.catalog().invalidate_pool(&self.agent);
        self.registry.maintenance.notify_one();
    }
}

impl Registry {
    pub fn credentials_change(self: &Arc<Self>, agent: AgentId) -> CredentialsChange {
        self.catalog().invalidate_pool(&agent);
        CredentialsChange {
            registry: Arc::clone(self),
            agent,
        }
    }

    pub fn acp_policy(&self) -> AcpLifecycle {
        self.configuration
            .read()
            .expect("configuration lock poisoned")
            .acp
            .clone()
    }

    /// # Errors
    /// Rejects an invalid policy before changing runtime behavior.
    pub fn set_acp_policy(&self, policy: AcpLifecycle) -> anyhow::Result<()> {
        let mut current = self
            .configuration
            .write()
            .expect("configuration lock poisoned");
        anyhow::ensure!(!self.stopping.is_cancelled(), "the daemon is stopping");
        let agents = current.catalog.registrations().cloned().collect::<Vec<_>>();
        policy.validate(&agents)?;
        current.acp = policy;
        self.maintenance.notify_one();
        Ok(())
    }

    /// Publishes policy and catalog as one generation after validating both.
    ///
    /// # Errors
    /// Keeps the last valid configuration intact on read, validation or installation errors.
    pub fn reload_configuration(
        &self,
        load: impl FnOnce() -> anyhow::Result<bas::config::Daemon>,
    ) -> anyhow::Result<bas::config::Daemon> {
        let mut current = self
            .configuration
            .write()
            .expect("configuration lock poisoned");
        anyhow::ensure!(!self.stopping.is_cancelled(), "the daemon is stopping");
        let settings = load()?;
        settings.acp.validate(&settings.agents)?;
        let catalog = Catalog::from_registrations(settings.agents.clone())?;
        if !current.catalog.registrations().eq(catalog.registrations()) {
            current
                .catalog
                .maintain_pools(&BTreeMap::new(), self.limits.startup_timeout);
            current.catalog = Arc::new(catalog);
        }
        current.acp = settings.acp.clone();
        self.maintenance.notify_one();
        Ok(settings)
    }

    /// Wake prewarming after actual use. With storage, the committed selection is authoritative,
    /// not RPC completion order; it also survives a cancelled response or daemon restart.
    pub fn record_agent_use(&self, id: AgentId) {
        if !self
            .catalog()
            .spec(&id)
            .is_some_and(|spec| spec.kind == Kind::Acp)
        {
            return;
        }
        if self.storage.is_none() {
            *self.last_agent.lock().expect("last agent lock poisoned") = Some(id);
        }
        self.maintenance.notify_one();
    }

    /// Called only after registry publication, so cancelled pending launches cannot select a pool.
    pub(super) async fn record_creation(&self, session: &Arc<Session>) {
        if let Some(storage) = &self.storage
            && session.backend_id.get().is_some()
        {
            let storage = Arc::clone(storage);
            let id = session.id();
            // The published session survives a cancelled response; its preference write must too.
            let write = tokio::spawn(async move {
                if let Err(error) = storage.record_created_session(id).await {
                    tracing::warn!(session = %id, %error, "cannot remember created ACP agent");
                }
            });
            if let Err(error) = write.await {
                tracing::warn!(session = %id, %error, "ACP selection writer failed");
            }
        }
        self.record_agent_use(session.meta().agent);
    }

    pub(super) async fn refresh_last_agent(&self) {
        let Some(storage) = &self.storage else {
            return;
        };
        match storage.last_used_agent().await {
            Ok(agent) => {
                *self.last_agent.lock().expect("last agent lock poisoned") = agent;
            }
            Err(error) => {
                tracing::warn!(%error, "cannot refresh last-used ACP agent; keeping last known pool target");
            }
        }
    }

    /// Pins the runtime before checking or restoring it, closing the eviction/dispatch race.
    ///
    /// # Errors
    /// Preserves the same recovery failures as `ensure_restored`.
    pub async fn use_backend(&self, session: &Arc<Session>) -> Result<BackendUse, CreateError> {
        let lease = session.use_backend();
        self.ensure_restored(session).await?;
        Ok(lease)
    }

    pub async fn maintain_backends(&self) {
        if self.stopping.is_cancelled() {
            return;
        }
        self.refresh_last_agent().await;
        let timeout = {
            let current = self
                .configuration
                .read()
                .expect("configuration lock poisoned");
            if self.stopping.is_cancelled() {
                return;
            }
            let last = self.last_agent.lock().expect("last agent lock poisoned");
            let last = last.as_ref().filter(|id| {
                current
                    .catalog
                    .spec(id)
                    .is_some_and(|spec| spec.kind == Kind::Acp)
            });
            let targets = current.acp.pool_targets(last);
            current
                .catalog
                .maintain_pools(&targets, self.limits.startup_timeout);
            current
                .acp
                .idle_timeout_secs
                .map(|seconds| Duration::from_secs(seconds.get()))
        };
        if let Some(timeout) = timeout {
            let sessions: Vec<_> = self.sessions.read().await.values().cloned().collect();
            futures::future::join_all(sessions.iter().map(|session| {
                session.reclaim_idle(
                    timeout,
                    Arc::clone(&self.permissions),
                    Arc::clone(&self.elicitations),
                )
            }))
            .await;
        }
    }

    pub async fn maintain(self: Arc<Self>) {
        loop {
            self.maintain_backends().await;
            tokio::select! {
                () = self.stopping.cancelled() => return,
                () = self.maintenance.notified() => {},
                () = tokio::time::sleep(MAINTENANCE_INTERVAL) => {},
            }
        }
    }
}
