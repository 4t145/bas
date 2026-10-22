use super::*;

impl Session {
    /// Reconcile one failed runtime with committed storage, without resending any prompt.
    ///
    /// # Errors
    /// Keeps the existing failure latched until a recovery write succeeds.
    pub(in crate::session) async fn recover_persistence(
        self: &Arc<Self>,
        gate: tokio::sync::OwnedMutexGuard<()>,
        capacity: usize,
    ) -> Result<(), SessionError> {
        self.owned(move |session| async move {
            let _restore = gate;
            let _transaction = session.transaction.lock().await;
            let storage = {
                let state = session.lock();
                if state.deleted || state.runtime.failure.is_none() {
                    return Err(SessionError::Inactive(state.meta.status));
                }
                if state.backend_users != 0 {
                    return Err(SessionError::RecoveryBusy);
                }
                state.storage.clone().ok_or(storage::Error::InvalidCommit(
                    "persistence recovery requires storage",
                ))?
            };
            let stored = storage
                .load(session.id())
                .await?
                .ok_or(storage::Error::NotFound(session.id()))?;
            if let Some(resume) = &stored.resume {
                let id = resume.acp_session_id()?;
                if session.backend_id.get().is_some_and(|known| known != id) {
                    return Err(storage::Error::Corrupt("backend session identity changed").into());
                }
                session.remember_backend_id(id.to_owned());
            }
            let mut meta = stored.meta;
            let mut kinds = Vec::new();
            if let Some(turn) = meta.active_turn {
                kinds.push(EventKind::TurnInterrupted {
                    turn,
                    reason: InterruptionReason::SessionStopped,
                });
            }
            let status = if meta.archived {
                SessionStatus::Stopped
            } else {
                match meta.status {
                    SessionStatus::Working => SessionStatus::Idle,
                    SessionStatus::Starting => SessionStatus::Failed,
                    status => status,
                }
            };
            // Even an idle session must prove it can commit before its failure is cleared.
            kinds.push(EventKind::StatusChanged { status });
            meta.subagents.invalidate_activity();
            let mut events = Vec::new();
            for kind in kinds {
                let event = Event::new(meta.id, meta.revision.next(), kind);
                meta.apply_event(&event);
                events.push(event);
            }
            storage.commit(meta.clone(), &events, None).await?;

            let mut state = session.lock();
            state.meta = meta;
            state.cursor = state.meta.revision;
            state.runtime = SessionRuntime::default();
            // Old subscribers retain their failed channel. New attachments start a new generation.
            state.failure = watch::channel(None).0;
            state.events = (!state.terminal()).then(|| broadcast::channel(capacity).0);
            state.active = None;
            state.agent = None;
            state.restored = false;
            state.closing = false;
            state.reloading = false;
            state.retiring = false;
            state.backend_generation += 1;
            state.last_backend_use = tokio::time::Instant::now();
            Ok(())
        })
        .await
    }
}
