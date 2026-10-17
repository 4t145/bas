use super::*;
use std::{path::PathBuf, time::Duration};

impl Session {
    /// # Errors
    /// Requires an idle, non-archived, capable backend. Rejections keep the old roots;
    /// unknown outcomes stop the runtime rather than continuing with uncertain access.
    pub async fn set_directories(
        self: &Arc<Self>,
        directories: Vec<PathBuf>,
        timeout: Duration,
    ) -> Result<SessionSnapshot, SessionError> {
        self.owned(move |session| async move {
            let _restore = session.restore_gate.lock().await;
            let response = {
                let _transaction = session.transaction.lock().await;
                let mut state = session.lock();
                state.require_ready()?;
                if state.active.is_some()
                    || state.meta.archived
                    || state.meta.status != SessionStatus::Idle
                {
                    return Err(SessionError::Inactive(state.meta.status));
                }
                if state.meta.additional_directories == directories {
                    return Ok(SessionSnapshot {
                        meta: state.meta.clone(),
                        runtime: state.runtime.clone(),
                    });
                }
                let response = state
                    .agent
                    .as_ref()
                    .expect("ready backend")
                    .set_directories(directories);
                state.reloading = true;
                response
            };
            let result = tokio::time::timeout(timeout, response)
                .await
                .unwrap_or_else(|_| {
                    Err(agent::Error::backend(
                        "acp",
                        "workspace reload timed out; outcome unknown",
                    ))
                });
            let _transaction = session.transaction.lock().await;
            session.lock().reloading = false;
            if let Err(error) = result {
                if matches!(error, agent::Error::Backend { .. }) {
                    session.fail_locked(error.to_string()).await?;
                }
                return Err(error.into());
            }
            session.check_health()?;
            Ok(session.snapshot())
        })
        .await
    }
}
