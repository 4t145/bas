//! Explicit restart is staged: rejected or cancelled loads leave the stopped record intact.

use super::*;
use tokio_util::task::AbortOnDropHandle;

impl Registry {
    /// # Errors
    /// Requires a healthy stopped ACP record and its original backend identity.
    /// Unsupported or failed restoration never creates a replacement session.
    pub async fn restart_session(&self, session: &Arc<Session>) -> Result<(), CreateError> {
        tokio::select! {
            biased;
            () = self.stopping.cancelled() => Err(CreateError::Stopping),
            result = tokio::time::timeout(self.limits.startup_timeout, self.restart_stopped(session)) => {
                result.map_err(|_| CreateError::Timeout)?
            }
        }
    }

    async fn restart_stopped(&self, session: &Arc<Session>) -> Result<(), CreateError> {
        // Also excludes delete, close, daemon shutdown, and concurrent restarts.
        let gate = Arc::clone(&session.restore_gate).lock_owned().await;
        session.check_restartable()?;
        let meta = session.meta();
        let catalog = self.catalog();
        let spec = catalog
            .spec(&meta.agent)
            .cloned()
            .ok_or_else(|| CreateError::UnknownAgent(meta.agent.clone()))?;
        if spec.kind != Kind::Acp {
            return Err(CreateError::NotResumable(
                "backend cannot reload a stopped session",
            ));
        }
        let backend_id = session
            .backend_id
            .get()
            .cloned()
            .ok_or(CreateError::NotResumable(
                "ACP session has no saved backend ID",
            ))?;
        let cwd = meta
            .cwd
            .clone()
            .ok_or(CreateError::NotResumable("session workspace is missing"))?;
        let (updates, mut inbox) = mpsc::channel(self.limits.live_capacity.get());
        // Existing history is authoritative. Acknowledge load replay without appending it
        // again, and hand the same inbox to the live session at the response barrier.
        let replay = AbortOnDropHandle::new(tokio::spawn(async move {
            while let Some(update) = inbox.recv().await {
                match update {
                    agent::Update::Commit { committed, .. } => {
                        let _ = committed.send(());
                    }
                    agent::Update::Barrier(ack) => {
                        let _ = ack.send(());
                        return Ok(inbox);
                    }
                    agent::Update::Failed(_) => {
                        return Err(CreateError::NotResumable("backend failed during restart"));
                    }
                    _ => {}
                }
            }
            Err(CreateError::NotResumable("restart update channel closed"))
        }));
        let agent = catalog
            .restore(
                LaunchRequest {
                    session: meta.id,
                    spec,
                    cwd,
                    host: Arc::clone(&self.host),
                    updates: updates.clone(),
                },
                backend_id,
            )
            .await?;
        let (ack, drained) = tokio::sync::oneshot::channel();
        updates
            .send(agent::Update::Barrier(ack))
            .await
            .map_err(|_| CreateError::NotResumable("restart listener closed"))?;
        drained
            .await
            .map_err(|_| CreateError::NotResumable("restart replay failed"))?;
        let inbox = replay
            .await
            .map_err(|_| CreateError::NotResumable("restart listener failed"))??;
        session
            .install_restarted(agent, inbox, self.limits.live_capacity.get(), gate)
            .await?;
        Ok(())
    }
}
