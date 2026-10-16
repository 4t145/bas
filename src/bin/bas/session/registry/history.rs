use super::*;

impl Registry {
    fn history_spec(&self, id: &AgentId) -> Result<Arc<agent::AgentSpec>, CreateError> {
        let catalog = self.catalog();
        let spec = catalog
            .spec(id)
            .cloned()
            .ok_or_else(|| CreateError::UnknownAgent(id.clone()))?;
        catalog.validate(&spec)?;
        if spec.kind != Kind::Acp {
            return Err(agent::Error::Unsupported {
                agent: id.clone(),
                operation: "Agent history",
            }
            .into());
        }
        Ok(spec)
    }

    /// # Errors
    /// Rejects unsupported Agents, unavailable processes, invalid pagination,
    /// shutdown and expired discovery deadlines.
    pub async fn list_agent_sessions(
        &self,
        agent: AgentId,
        cwd: Option<PathBuf>,
    ) -> Result<agent::acp::history::Listing, CreateError> {
        let spec = self.history_spec(&agent)?;
        tokio::select! {
            biased;
            () = self.stopping.cancelled() => Err(CreateError::Stopping),
            result = tokio::time::timeout(self.limits.startup_timeout, agent::acp::history::list(&spec, cwd)) => {
                result.map_err(|_| CreateError::Timeout)?.map_err(Into::into)
            }
        }
    }

    pub async fn imported_sessions(
        &self,
        agent: &AgentId,
    ) -> std::collections::HashMap<String, String> {
        self.sessions
            .read()
            .await
            .values()
            .filter_map(|session| {
                (session.meta().agent == *agent)
                    .then(|| {
                        session
                            .backend_id
                            .get()
                            .map(|id| (id.clone(), session.id().to_string()))
                    })
                    .flatten()
            })
            .collect()
    }

    /// # Errors
    /// Never falls back to session/new. Failed imports are not registered; retries
    /// for an already imported Agent ID return the same public Bas session.
    pub async fn import_agent_session(
        &self,
        request: CreateRequest,
        backend_id: String,
    ) -> Result<Arc<Session>, CreateError> {
        let _gate = self.history_gate.lock().await;
        if backend_id.is_empty() {
            return Err(CreateError::NotResumable("Agent session ID is empty"));
        }
        if let Some(existing) = self.find_imported(&request.agent, &backend_id).await {
            return Ok(existing);
        }
        self.history_spec(&request.agent)?;
        tokio::select! {
            biased;
            () = self.stopping.cancelled() => Err(CreateError::Stopping),
            result = tokio::time::timeout(self.limits.startup_timeout, self.launch(request, Some(backend_id))) => {
                result.map_err(|_| CreateError::Timeout)?
            }
        }
    }

    async fn find_imported(&self, agent: &AgentId, backend_id: &str) -> Option<Arc<Session>> {
        self.sessions
            .read()
            .await
            .values()
            .find(|session| {
                session.meta().agent == *agent
                    && session.backend_id.get().is_some_and(|id| id == backend_id)
            })
            .cloned()
    }

    /// # Errors
    /// Refuses to delete history used by an open Bas session. The caller must
    /// close that session explicitly; deleting Agent history never deletes Bas data.
    pub async fn delete_agent_session(
        &self,
        agent: AgentId,
        backend_id: String,
    ) -> Result<(), CreateError> {
        let _gate = self.history_gate.lock().await;
        if let Some(session) = self.find_imported(&agent, &backend_id).await {
            let _session_gate = session.restore_gate.lock().await;
            if !matches!(
                session.meta().status,
                SessionStatus::Stopped | SessionStatus::Failed
            ) {
                return Err(CreateError::NotResumable(
                    "close the imported Bas session before deleting Agent history",
                ));
            }
        }
        let spec = self.history_spec(&agent)?;
        tokio::select! {
            biased;
            () = self.stopping.cancelled() => Err(CreateError::Stopping),
            result = tokio::time::timeout(self.limits.startup_timeout, agent::acp::history::delete(&spec, backend_id)) => {
                result.map_err(|_| CreateError::Timeout)?.map_err(Into::into)
            }
        }
    }
}
