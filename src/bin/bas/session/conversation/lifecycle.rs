use std::{sync::Arc, time::Duration};

use super::*;

#[cfg(test)]
mod tests;

/// Pins the runtime from before restoration through the caller's backend operation.
/// A client subscription does not acquire this lease.
#[must_use]
pub struct BackendUse(Arc<Session>);

impl Drop for BackendUse {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.backend_users -= 1;
        state.last_backend_use = tokio::time::Instant::now();
    }
}

impl Session {
    pub(in crate::session) fn use_backend(self: &Arc<Self>) -> BackendUse {
        let mut state = self.lock();
        state.backend_users += 1;
        state.last_backend_use = tokio::time::Instant::now();
        BackendUse(Arc::clone(self))
    }

    /// Releases only a restorable, quiescent runtime; history and live subscribers survive.
    /// The backend seals callback admission before relinquishing the process.
    pub(in crate::session) async fn reclaim_idle(
        self: &Arc<Self>,
        timeout: Duration,
        permissions: Arc<agent::permissions::Permissions>,
        elicitations: Arc<agent::elicitation::Elicitations>,
    ) -> bool {
        self.owned(move |session| async move {
            let Ok(_restore) = session.restore_gate.try_lock() else {
                return false;
            };
            let Ok(transaction) = session.transaction.try_lock() else {
                return false;
            };
            let mut agent = {
                let mut state = session.lock();
                if state.terminal()
                    || state.deleted
                    || state.closing
                    || state.reloading
                    || !state.restored
                    || state.storage.is_none()
                    || session.backend_id.get().is_none()
                    || state.meta.status != SessionStatus::Idle
                    || state.active.is_some()
                    || state.backend_users != 0
                    || state.last_backend_use.elapsed() < timeout
                    || state
                        .meta
                        .subagents
                        .iter()
                        .any(|child| !matches!(child.state, Some(acp::StateUpdate::Idle(_))))
                    || permissions.has_pending(state.meta.id)
                    || elicitations.has_pending(state.meta.id)
                {
                    return false;
                }
                let Some(agent) = state.agent.as_mut() else {
                    return false;
                };
                if !agent.try_idle_reclaim(timeout) {
                    return false;
                }
                state.restored = false;
                state.retiring = true;
                state.agent.take().expect("claimed idle backend")
            };
            // Callback admission is sealed, but previously queued events still belong to history.
            // Restore waits on _restore while the old listener commits through its barrier.
            drop(transaction);
            agent.drain_updates().await;
            let _transaction = session.transaction.lock().await;
            {
                let mut state = session.lock();
                state.retiring = false;
                state.backend_generation += 1;
            }
            let listener = session
                .updates_task
                .lock()
                .expect("update task lock")
                .take();
            if let Some(listener) = listener {
                listener.abort();
                let _ = listener.await;
            }
            agent.release().await;
            tracing::info!(session = %session.id(), "released idle ACP runtime");
            true
        })
        .await
    }
}
