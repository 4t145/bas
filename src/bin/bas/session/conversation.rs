use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

use bas::acp;
use bas::model::{
    Cursor, Event, EventKind, InterruptionReason, PromptResult, RuntimeFailure, SessionId,
    SessionMeta, SessionRuntime, SessionSnapshot, SessionStatus, TurnId, TurnReceipt,
};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::task::AbortOnDropHandle;

#[cfg(test)]
use super::Page;
use super::{AttachOptions, Attachment, HistoryError, Limits, history::Ring};
use crate::agent::{self, Agent, TurnUpdate, Update};
use crate::storage::{self, Storage};

mod directories;
mod submission;
pub use submission::SubmissionError;

/// A detached snapshot; notification delivery never retains the session.
#[derive(Clone, Debug)]
pub struct Completion {
    pub session: SessionMeta,
    pub outcome: CompletionOutcome,
}

#[derive(Clone, Debug)]
pub enum CompletionOutcome {
    Native(PromptResult),
    Interrupted(InterruptionReason),
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Runtime(#[from] RuntimeFailure),
    #[error("session is {0:?}")]
    Inactive(SessionStatus),
    #[error(transparent)]
    Agent(#[from] agent::Error),
    #[error(transparent)]
    Storage(#[from] storage::Error),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error("session commit worker failed: {0}")]
    CommitTask(#[from] tokio::task::JoinError),
}

pub struct Session {
    state: Mutex<State>,
    transaction: tokio::sync::Mutex<()>,
    updates_task: Mutex<Option<AbortOnDropHandle<()>>>,
    pub(super) restore_gate: Arc<tokio::sync::Mutex<()>>,
    pub(super) backend_id: std::sync::OnceLock<String>,
}

/// A connected foreground viewer; it never retains or stops the backend.
pub struct Foreground(Weak<Session>);

impl Drop for Foreground {
    fn drop(&mut self) {
        if let Some(session) = self.0.upgrade() {
            session.lock().foreground_viewers -= 1;
        }
    }
}

/// All mutations and broadcasts share one lock. Backend methods only enqueue work.
struct State {
    meta: SessionMeta,
    runtime: SessionRuntime,
    failure: watch::Sender<Option<RuntimeFailure>>,
    history: Ring,
    cursor: Cursor,
    active: Option<TurnId>,
    agent: Option<Box<dyn Agent>>,
    events: Option<broadcast::Sender<Event>>,
    completions: Option<mpsc::Sender<Completion>>,
    foreground_viewers: usize,
    startup_config: Option<acp::ConfigOptionUpdate>,
    startup_mode: Option<acp::CurrentModeUpdate>,
    storage: Option<Arc<dyn Storage>>,
    // Used only by sessions without a Storage backend; follows the session lifetime.
    acceptances: HashMap<TurnId, storage::PromptAcceptance>,
    restored: bool,
    deleted: bool,
    closing: bool,
    reloading: bool,
}

impl Session {
    #[cfg(test)]
    pub(super) fn new(
        meta: SessionMeta,
        limits: Limits,
        completions: Option<mpsc::Sender<Completion>>,
    ) -> Arc<Self> {
        Self::with_storage(meta, limits, completions, None, true)
    }

    pub(super) fn with_storage(
        meta: SessionMeta,
        limits: Limits,
        completions: Option<mpsc::Sender<Completion>>,
        storage: Option<Arc<dyn Storage>>,
        restored: bool,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(limits.live_capacity.get());
        let cursor = meta.revision;
        let events = (!matches!(meta.status, SessionStatus::Stopped | SessionStatus::Failed))
            .then_some(events);
        Arc::new(Self {
            state: Mutex::new(State {
                meta,
                runtime: SessionRuntime::default(),
                failure: watch::channel(None).0,
                history: Ring::new(limits.history_capacity),
                cursor,
                active: None,
                agent: None,
                events,
                completions,
                foreground_viewers: 0,
                startup_config: None,
                startup_mode: None,
                storage,
                acceptances: HashMap::new(),
                restored,
                deleted: false,
                closing: false,
                reloading: false,
            }),
            transaction: tokio::sync::Mutex::new(()),
            updates_task: Mutex::new(None),
            restore_gate: Arc::new(tokio::sync::Mutex::new(())),
            backend_id: std::sync::OnceLock::new(),
        })
    }

    pub(super) fn remember_backend_id(&self, id: String) {
        let _ = self.backend_id.set(id);
    }

    pub(super) fn is_restored(&self) -> bool {
        self.lock().restored
    }

    /// # Errors
    /// A failed recovery commit terminates this loaded runtime without altering its metadata.
    pub(super) async fn recover(self: &Arc<Self>) -> Result<(), SessionError> {
        self.owned(|session| async move {
            let _gate = session.transaction.lock().await;
            let mut meta = session.meta();
            meta.subagents.invalidate_activity();
            let events = if meta.archived && meta.status != SessionStatus::Stopped {
                let mut events = Vec::new();
                if let Some(turn) = meta.active_turn {
                    events.push(EventKind::TurnInterrupted {
                        turn,
                        reason: InterruptionReason::SessionStopped,
                    });
                }
                events.push(EventKind::StatusChanged {
                    status: SessionStatus::Stopped,
                });
                events
            } else {
                match meta.status {
                    SessionStatus::Starting => vec![EventKind::Failed {
                        message: "daemon stopped during session startup".to_owned(),
                    }],
                    SessionStatus::Working => match meta.active_turn {
                        Some(turn) => vec![EventKind::TurnInterrupted {
                            turn,
                            reason: InterruptionReason::DaemonRestarted,
                        }],
                        None => {
                            meta.set_status(SessionStatus::Idle);
                            Vec::new()
                        }
                    },
                    _ => Vec::new(),
                }
            };
            session.publish_with_meta_locked(events, &mut meta).await?;
            let mut state = session.lock();
            if state.terminal() {
                state.events.take();
            }
            Ok(())
        })
        .await
    }

    pub(super) async fn restore_agent(
        self: &Arc<Self>,
        agent: Box<dyn Agent>,
        restore_gate: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<(), SessionError> {
        self.owned(move |session| async move {
            // Retain exclusivity with the commit even if its caller times out.
            let _restore_gate = restore_gate;
            let _gate = session.transaction.lock().await;
            let (mut meta, updates) = {
                let state = session.lock();
                if state.terminal() || state.deleted {
                    state.check_health()?;
                    return Err(SessionError::Inactive(state.meta.status));
                }
                let mut meta = state.meta.clone();
                meta.initialization = agent.initialization();
                meta.subagents = agent.subagents();
                meta.initialization_raw = agent.initialization_raw();
                meta.acp = agent.session_info().map(|mut info| {
                    info.session_id = meta.id.to_string().into();
                    info
                });
                let mut updates = Vec::new();
                if let Some(info) = &meta.acp {
                    if let Some(options) = &info.config_options
                        && state
                            .meta
                            .acp
                            .as_ref()
                            .and_then(|info| info.config_options.as_ref())
                            != Some(options)
                    {
                        updates.push(EventKind::Acp(acp::SessionUpdate::ConfigOptionUpdate(
                            acp::ConfigOptionUpdate::new(options.clone()),
                        )));
                    }
                    if let Some(modes) = &info.modes
                        && state
                            .meta
                            .acp
                            .as_ref()
                            .and_then(|info| info.modes.as_ref())
                            .map(|modes| &modes.current_mode_id)
                            != Some(&modes.current_mode_id)
                    {
                        updates.push(EventKind::Acp(acp::SessionUpdate::CurrentModeUpdate(
                            acp::CurrentModeUpdate::new(modes.current_mode_id.clone()),
                        )));
                    }
                }
                (meta, updates)
            };
            session.publish_with_meta_locked(updates, &mut meta).await?;
            if let Some(id) = agent.acp_session_id() {
                session.remember_backend_id(id);
            }
            let mut state = session.lock();
            state.agent = Some(agent);
            state.restored = true;
            Ok(())
        })
        .await
    }

    pub(super) fn check_restartable(&self) -> Result<(), SessionError> {
        let state = self.lock();
        state.check_health()?;
        if state.deleted || state.meta.status != SessionStatus::Stopped {
            return Err(SessionError::Inactive(state.meta.status));
        }
        Ok(())
    }

    pub(super) async fn install_restarted(
        self: &Arc<Self>,
        agent: Box<dyn Agent>,
        inbox: mpsc::Receiver<Update>,
        capacity: usize,
        restore_gate: tokio::sync::OwnedMutexGuard<()>,
        unarchive: bool,
    ) -> Result<SessionSnapshot, SessionError> {
        self.owned(move |session| async move {
            let _restore = restore_gate;
            let _transaction = session.transaction.lock().await;
            let mut meta = {
                let state = session.lock();
                state.check_health()?;
                if state.deleted || state.meta.status != SessionStatus::Stopped {
                    return Err(SessionError::Inactive(state.meta.status));
                }
                state.meta.clone()
            };
            meta.initialization = agent.initialization();
            meta.initialization_raw = agent.initialization_raw();
            meta.subagents = agent.subagents();
            meta.acp = agent.session_info().map(|mut info| {
                info.session_id = meta.id.to_string().into();
                info
            });
            let mut events = Vec::new();
            if unarchive {
                events.push(EventKind::ArchiveChanged { archived: false });
            }
            events.push(EventKind::StatusChanged {
                status: SessionStatus::Idle,
            });
            session.publish_with_meta_locked(events, &mut meta).await?;
            {
                let mut state = session.lock();
                state.agent = Some(agent);
                state.restored = true;
                state.closing = false;
                state.events = Some(broadcast::channel(capacity).0);
            }
            session.listen(inbox);
            Ok(session.snapshot())
        })
        .await
    }

    /// Stop the process without recording a terminal event: a durable session
    /// will be restarted by the next daemon and restored on its next prompt.
    pub(super) async fn suspend(&self) {
        let _gate = self.transaction.lock().await;
        let agent = {
            let mut state = self.lock();
            state.restored = false;
            state.events.take();
            state.agent.take()
        };
        let task = self.updates_task.lock().expect("update task lock").take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        if let Some(mut agent) = agent {
            agent.shutdown().await;
        }
    }

    /// Removes the durable record while excluding concurrent session mutations.
    /// Backend updates arriving after deletion are discarded until shutdown.
    ///
    /// # Errors
    /// Returns a storage error without changing the live session if deletion fails.
    pub(super) async fn delete_storage(self: &Arc<Self>) -> Result<(), storage::Error> {
        self.owned(|session| async move {
            let _gate = session.transaction.lock().await;
            let (storage, id) = {
                let state = session.lock();
                (state.storage.clone(), state.meta.id)
            };
            if let Some(storage) = storage {
                storage.delete(id).await?;
            }
            let mut state = session.lock();
            state.restored = false;
            state.deleted = true;
            state.acceptances.clear();
            state.events.take();
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.lock()
            .events
            .as_ref()
            .map_or(0, broadcast::Sender::receiver_count)
    }

    pub fn id(&self) -> SessionId {
        self.lock().meta.id
    }

    pub fn foreground(self: &Arc<Self>) -> Foreground {
        self.lock().foreground_viewers += 1;
        Foreground(Arc::downgrade(self))
    }

    #[cfg(test)]
    pub(crate) fn foreground_viewers(&self) -> usize {
        self.lock().foreground_viewers
    }
    pub fn meta(&self) -> SessionMeta {
        self.lock().meta.clone()
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let state = self.lock();
        SessionSnapshot {
            meta: state.meta.clone(),
            runtime: state.runtime.clone(),
        }
    }

    pub(super) fn check_health(&self) -> Result<(), SessionError> {
        self.lock().check_health()
    }

    /// Changes only archival metadata, including for starting or terminal sessions.
    /// Repeating the current value does not publish an event or advance revision.
    #[cfg(test)]
    pub async fn set_archived(self: &Arc<Self>, archived: bool) -> SessionMeta {
        self.try_set_archived(archived)
            .await
            .expect("archival metadata must be committed")
            .meta
    }

    /// Captures the operation's snapshot before another mutation can overtake its reply.
    ///
    /// # Errors
    /// Rejects failed/deleted instances or a failed persistence commit.
    #[cfg(test)]
    pub async fn try_set_archived(
        self: &Arc<Self>,
        archived: bool,
    ) -> Result<SessionSnapshot, SessionError> {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            let changed = {
                let state = session.lock();
                state.check_health()?;
                if state.deleted {
                    return Err(SessionError::Inactive(SessionStatus::Stopped));
                }
                state.meta.archived != archived
            };
            if changed {
                session
                    .publish_locked([EventKind::ArchiveChanged { archived }])
                    .await?;
            }
            Ok(session.snapshot())
        })
        .await
    }

    pub(super) fn listen(self: &Arc<Self>, mut inbox: mpsc::Receiver<Update>) {
        let state = self.lock();
        if state.terminal() || state.deleted {
            return;
        }
        let weak = Arc::downgrade(self);
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            while let Some(update) = inbox.recv().await {
                let Some(session) = weak.upgrade() else {
                    return;
                };
                session.apply(update).await;
                if session.lock().terminal() {
                    return;
                }
            }
            if let Some(session) = weak.upgrade()
                && session.is_restored()
            {
                session
                    .fail("backend update channel closed".to_owned())
                    .await;
            }
        }));
        *self
            .updates_task
            .lock()
            .expect("update task lock is not poisoned") = Some(task);
        drop(state);
    }

    /// # Errors
    /// Returns an error if initialization already failed or shutdown started.
    pub(super) async fn ready(self: &Arc<Self>, agent: Box<dyn Agent>) -> Result<(), SessionError> {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            let mut meta = {
                let state = session.lock();
                if state.terminal() {
                    state.check_health()?;
                    return Err(SessionError::Inactive(state.meta.status));
                }
                let mut meta = state.meta.clone();
                meta.initialization = agent.initialization();
                meta.initialization_raw = agent.initialization_raw();
                meta.acp = agent.session_info().map(|mut info| {
                    info.session_id = meta.id.to_string().into();
                    info
                });
                if let Some(update) = &state.startup_config {
                    meta.apply_acp(&acp::SessionUpdate::ConfigOptionUpdate(update.clone()));
                }
                if let Some(update) = &state.startup_mode {
                    meta.apply_acp(&acp::SessionUpdate::CurrentModeUpdate(update.clone()));
                }
                meta
            };
            session
                .publish_with_meta_locked(
                    [EventKind::StatusChanged {
                        status: SessionStatus::Idle,
                    }],
                    &mut meta,
                )
                .await?;
            if let Some(id) = agent.acp_session_id() {
                session.remember_backend_id(id);
            }
            let mut state = session.lock();
            state.startup_config = None;
            state.startup_mode = None;
            state.agent = Some(agent);
            Ok(())
        })
        .await
    }

    /// Snapshots history and subscribes atomically with respect to publication.
    ///
    /// # Errors
    /// Rejects expired or future cursors instead of silently skipping history.
    #[cfg(test)]
    pub async fn attach(
        self: &Arc<Self>,
        options: AttachOptions,
    ) -> Result<Attachment, HistoryError> {
        self.try_attach(options).await.map_err(|error| match error {
            SessionError::History(error) => error,
            _ => panic!("memory-backed test session cannot have storage errors"),
        })
    }

    pub async fn try_attach(&self, options: AttachOptions) -> Result<Attachment, SessionError> {
        let (storage, id, cursor, events, mut failure) = {
            let state = self.lock();
            state.check_health()?;
            state.check_cursor(options.cursor)?;
            if state.storage.is_none() {
                let backlog = match options.cursor {
                    Some(cursor) => state.history.page(Some(cursor), usize::MAX)?.events,
                    None => state.history.tail(options.replay),
                };
                return Ok(Attachment::new(
                    backlog,
                    options.cursor.unwrap_or(Cursor::START),
                    state.events.as_ref().map(broadcast::Sender::subscribe),
                    state.failure.subscribe(),
                ));
            }
            (
                state.storage.clone().expect("storage checked"),
                state.meta.id,
                state.cursor,
                state.events.as_ref().map(broadcast::Sender::subscribe),
                state.failure.subscribe(),
            )
        };
        if options.cursor.is_none() && !storage.is_persistent() {
            let backlog = tokio::select! {
                biased;
                error = super::attachment::wait_failure(&mut failure) => return Err(error.into()),
                result = storage.tail(id, options.replay) => result?,
            };
            return Ok(Attachment::new(backlog, Cursor::START, events, failure));
        }
        let after = options.cursor.unwrap_or_else(|| {
            Cursor::from_raw(cursor.raw().saturating_sub(options.replay as u64))
        });
        if options.cursor.is_some() {
            tokio::select! {
                biased;
                error = super::attachment::wait_failure(&mut failure) => return Err(error.into()),
                result = storage.page(id, Some(after), 1) => { result?; }
            }
        }
        Ok(Attachment::persistent(
            after, cursor, storage, id, events, failure,
        ))
    }

    /// # Errors
    /// Rejects expired/future cursors and a zero page size.
    #[cfg(test)]
    pub fn page(&self, after: Option<Cursor>, limit: usize) -> Result<Page, HistoryError> {
        let state = self.lock();
        state.check_cursor(after)?;
        state.history.page(after, limit)
    }

    /// Visits history under one snapshot lock without cloning an unbounded page.
    ///
    /// The visitor must not re-enter the session. Its boolean argument indicates
    /// further history even beyond `limit`; returning false stops visitation.
    ///
    /// # Errors
    /// Rejects expired/future cursors and a zero page size.
    #[cfg(test)]
    pub async fn visit_history(
        &self,
        after: Option<Cursor>,
        limit: usize,
        visitor: impl FnMut(&Event, bool) -> bool,
    ) -> Result<(), HistoryError> {
        self.try_visit_history(after, limit, visitor)
            .await
            .map_err(|error| match error {
                SessionError::History(error) => error,
                _ => panic!("memory-backed test session cannot have storage errors"),
            })
    }

    pub async fn try_visit_history(
        &self,
        after: Option<Cursor>,
        limit: usize,
        visitor: impl FnMut(&Event, bool) -> bool,
    ) -> Result<(), SessionError> {
        let (storage, id) = {
            let state = self.lock();
            state.check_cursor(after)?;
            if state.storage.is_none() {
                return state
                    .history
                    .visit(after, limit, visitor)
                    .map_err(SessionError::History);
            }
            (
                state.storage.clone().expect("storage checked"),
                state.meta.id,
            )
        };
        if limit == 0 {
            return Err(SessionError::History(HistoryError::EmptyPage));
        }
        // The wire encoder may stop earlier on byte budget. Bound the
        // synchronous storage allocation independently of client input.
        let page = storage.page(id, after, limit.min(1024)).await?;
        let mut visitor = visitor;
        let len = page.events.len();
        for (index, event) in page.events.iter().enumerate() {
            if !visitor(event, index + 1 < len || page.next.is_some()) {
                break;
            }
        }
        Ok(())
    }

    /// Accepts exactly one turn and commits the user message before its updates.
    ///
    /// # Errors
    /// Reports explicit rejection or unknown acceptance; backend failures after acceptance are events.
    #[cfg(test)]
    pub async fn prompt(
        self: &Arc<Self>,
        content: Vec<acp::ContentBlock>,
    ) -> Result<TurnId, SubmissionError> {
        self.accept_prompt(bas::model::PromptSubmission::new(acp::PromptRequest::new(
            self.id().to_string(),
            content,
        )))
        .await
        .map(|receipt| receipt.turn)
    }

    /// # Errors
    /// Rejects inactive sessions or reports a backend cancellation failure.
    #[cfg(test)]
    pub async fn cancel(self: &Arc<Self>) -> Result<(), SessionError> {
        self.cancel_acp(acp::CancelNotification::new(self.id().to_string()))
            .await
    }

    /// # Errors
    /// Rejects inactive sessions or reports a backend cancellation failure.
    pub async fn cancel_acp(
        self: &Arc<Self>,
        request: acp::CancelNotification,
    ) -> Result<(), SessionError> {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            let result = {
                let mut state = session.lock();
                state.require_ready()?;
                if state.active.is_none() {
                    return Ok(());
                }
                state.agent.as_mut().expect("ready backend").cancel(request)
            };
            if let Err(error) = result {
                if matches!(error, agent::Error::Backend { .. }) {
                    session.fail_locked(error.to_string()).await?;
                }
                return Err(error.into());
            }
            Ok(())
        })
        .await
    }

    /// # Errors
    /// Returns backend rejection or an inactive session error.
    pub async fn set_config_option(
        &self,
        request: acp::SetSessionConfigOptionRequest,
    ) -> Result<acp::SetSessionConfigOptionResponse, SessionError> {
        let response = {
            let state = self.lock();
            state.require_ready()?;
            state
                .agent
                .as_ref()
                .expect("ready backend")
                .set_config_option(request)
        }
        .await;
        self.check_health()?;
        Ok(response?)
    }

    /// # Errors
    /// Returns backend rejection or an inactive session error.
    pub async fn set_mode(
        &self,
        request: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, SessionError> {
        let response = {
            let state = self.lock();
            state.require_ready()?;
            state
                .agent
                .as_ref()
                .expect("ready backend")
                .set_mode(request)
        }
        .await;
        self.check_health()?;
        Ok(response?)
    }

    /// Titles are daemon metadata, including for backends without a rename operation.
    ///
    /// # Errors
    /// Rejects inactive sessions.
    pub async fn set_title(self: &Arc<Self>, title: &str) -> Result<(), SessionError> {
        let title = title.to_owned();
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            let changed = {
                let state = session.lock();
                state.check_health()?;
                if state.deleted
                    || !matches!(
                        state.meta.status,
                        SessionStatus::Idle | SessionStatus::Working
                    )
                {
                    return Err(SessionError::Inactive(state.meta.status));
                }
                state.meta.title.as_ref() != Some(&title)
            };
            if changed {
                session
                    .publish_locked([EventKind::TitleChanged { title }])
                    .await?;
            }
            Ok(())
        })
        .await
    }

    /// # Errors
    /// Keeps the session intact if advertised standard closure is rejected.
    /// Transport failures release the uncertain runtime, without claiming remote success.
    /// Success preserves Bas history but stops this runtime.
    pub async fn close_backend(self: &Arc<Self>) -> Result<(), SessionError> {
        self.stop(false).await.map(|_| ())
    }

    pub(super) async fn archive(self: &Arc<Self>) -> Result<SessionSnapshot, SessionError> {
        self.stop(true).await
    }

    async fn stop(self: &Arc<Self>, archive: bool) -> Result<SessionSnapshot, SessionError> {
        self.owned(move |session| async move {
            let _restore = session.restore_gate.lock().await;
            let close = {
                let _transaction = session.transaction.lock().await;
                let mut state = session.lock();
                state.check_health()?;
                if state.deleted {
                    return Err(SessionError::Inactive(state.meta.status));
                }
                let supports_close = state
                    .agent
                    .as_ref()
                    .and_then(|agent| agent.initialization())
                    .is_some_and(|init| {
                        init.agent_capabilities.session_capabilities.close.is_some()
                    });
                state.closing = true;
                supports_close.then(|| {
                    state
                        .agent
                        .as_ref()
                        .expect("close-capable agent")
                        .close_session()
                })
            };
            // Do not hold the commit lock while the Agent flushes final updates.
            if let Some(close) = close
                && let Err(error) = close.await
            {
                session.lock().closing = false;
                if matches!(error, agent::Error::Backend { .. }) {
                    session.shutdown_owned().await;
                }
                return Err(error.into());
            }
            session.shutdown_owned().await;
            let _transaction = session.transaction.lock().await;
            session.check_health()?;
            let meta = session.meta();
            let mut events = Vec::new();
            if meta.status != SessionStatus::Stopped {
                events.push(EventKind::StatusChanged {
                    status: SessionStatus::Stopped,
                });
            }
            if archive && !meta.archived {
                events.push(EventKind::ArchiveChanged { archived: true });
            }
            session.publish_locked(events).await?;
            Ok(session.snapshot())
        })
        .await
    }

    /// Closes the event stream before awaiting backend cleanup. Safe to call repeatedly.
    pub async fn shutdown(self: &Arc<Self>) {
        let session = Arc::clone(self);
        self.owned(move |_| async move { session.shutdown_owned().await })
            .await;
    }

    async fn shutdown_owned(&self) {
        let _gate = self.transaction.lock().await;
        let needs_shutdown = !self.lock().terminal();
        if needs_shutdown
            && let Err(error) = self
                .interrupt_locked(
                    InterruptionReason::SessionStopped,
                    Some(EventKind::StatusChanged {
                        status: SessionStatus::Stopped,
                    }),
                )
                .await
        {
            tracing::error!(%error, "could not commit session shutdown status");
        }
        let agent = {
            let mut state = self.lock();
            state.events.take();
            state.agent.take()
        };
        let task = self
            .updates_task
            .lock()
            .expect("update task lock is not poisoned")
            .take();
        let stop_updates = async {
            if let Some(task) = task {
                task.abort();
                let _ = task.await;
            }
        };
        let stop_agent = async {
            if let Some(mut agent) = agent {
                agent.shutdown().await;
            }
        };
        tokio::join!(stop_updates, stop_agent);
    }

    pub(super) async fn apply(self: &Arc<Self>, update: Update) {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            if session.lock().terminal() || session.lock().deleted {
                return;
            }
            if let Update::Barrier(ack) = update {
                let _ = ack.send(());
                // The restore caller holds this gate until the load response has
                // been committed and the agent installed. Keep later updates in
                // the inbox while normal publication is still disabled.
                drop(_gate);
                let _restore = session.restore_gate.lock().await;
                return;
            }
            if !session.lock().restored {
                if let Update::Commit { committed, .. } = update {
                    let _ = committed.send(());
                }
                return;
            }
            let result = match update {
                Update::Acp(update) => session.publish_locked([EventKind::Acp(update)]).await,
                Update::Subagent(notification) => {
                    session
                        .publish_locked([EventKind::Subagent(notification)])
                        .await
                }
                Update::Commit { update, committed } => {
                    let result = session.publish_locked([EventKind::Acp(update)]).await;
                    if result.is_ok() {
                        let _ = committed.send(());
                    }
                    result
                }
                Update::Workspace {
                    directories,
                    mut config,
                    subagents,
                    committed,
                } => {
                    config.session_id = session.id().to_string().into();
                    let result = session
                        .publish_locked([EventKind::WorkspaceChanged {
                            directories,
                            config,
                            subagents,
                        }])
                        .await;
                    if result.is_ok() {
                        let _ = committed.send(());
                    }
                    result
                }
                Update::Failed(message) => session.fail_locked(message).await,
                Update::Turn { turn, update } if session.lock().active == Some(turn) => {
                    match update {
                        TurnUpdate::Acp(update) => {
                            session.publish_locked([EventKind::Acp(update)]).await
                        }
                        TurnUpdate::Finished(result) => session.finish_locked(result).await,
                        TurnUpdate::CancelledBeforeDispatch => {
                            session
                                .interrupt_locked(InterruptionReason::CancelledBeforeDispatch, None)
                                .await
                        }
                    }
                }
                Update::Turn { .. } => Ok(()),
                Update::Barrier(_) => Ok(()),
            };
            if let Err(error) = result {
                let state = session.lock();
                tracing::error!(session = %state.meta.id, %error, "session storage commit failed");
            }
        })
        .await;
    }

    pub(super) async fn fail(self: &Arc<Self>, message: String) {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            if let Err(error) = session.fail_locked(message).await {
                tracing::error!(%error, "could not commit backend failure");
            }
        })
        .await;
    }

    /// Once queued, the operation owns the session through its commit and all
    /// subsequent side effects, even if its caller (including an RPC) vanishes.
    async fn owned<T, F, Fut>(self: &Arc<Self>, operation: F) -> T
    where
        T: Send + 'static,
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let session = Arc::clone(self);
        tokio::spawn(operation(session))
            .await
            .expect("session operation panicked")
    }

    async fn publish_locked(
        &self,
        kinds: impl IntoIterator<Item = EventKind>,
    ) -> Result<(), SessionError> {
        let mut meta = self.meta();
        self.publish_with_meta_locked(kinds, &mut meta).await
    }

    async fn publish_with_meta_locked(
        &self,
        kinds: impl IntoIterator<Item = EventKind>,
        meta: &mut SessionMeta,
    ) -> Result<(), SessionError> {
        self.publish_submission_locked(kinds, meta, None).await
    }

    async fn publish_submission_locked(
        &self,
        kinds: impl IntoIterator<Item = EventKind>,
        meta: &mut SessionMeta,
        acceptance: Option<storage::PromptAcceptance>,
    ) -> Result<(), SessionError> {
        let (events, storage) = {
            let state = self.lock();
            state.check_health()?;
            let mut events = Vec::new();
            for kind in kinds {
                let mut event = Event::new(meta.id, Cursor::START, kind);
                if event.is_transient() {
                    if let Some(sender) = &state.events {
                        let _ = sender.send(event);
                    }
                    continue;
                }
                event.cursor = events
                    .last()
                    .map_or(state.cursor.next(), |event: &Event| event.cursor.next());
                meta.apply_event(&event);
                events.push(event);
            }
            (events, state.storage.clone())
        };
        if events.is_empty() && *meta == self.meta() {
            return Ok(());
        }
        if let Some(storage) = storage
            && let Err(error) = storage.commit(meta.clone(), &events, acceptance).await
        {
            return Err(self.persistence_failed(error).into());
        }
        let mut state = self.lock();
        state.meta = meta.clone();
        if state.storage.is_none()
            && let Some(acceptance) = acceptance
        {
            state
                .acceptances
                .insert(acceptance.receipt.turn, acceptance);
        }
        for event in events {
            state.cursor = event.cursor;
            if state.meta.status == SessionStatus::Starting {
                match &event.kind {
                    EventKind::Acp(acp::SessionUpdate::ConfigOptionUpdate(update)) => {
                        state.startup_config = Some(update.clone());
                    }
                    EventKind::Acp(acp::SessionUpdate::CurrentModeUpdate(update)) => {
                        state.startup_mode = Some(update.clone());
                    }
                    _ => {}
                }
            }
            state.history.push(event.clone());
            if let Some(sender) = &state.events {
                let _ = sender.send(event);
            }
        }
        Ok(())
    }

    fn persistence_failed(&self, error: storage::Error) -> RuntimeFailure {
        let mut state = self.lock();
        let failure = state
            .runtime
            .failure
            .get_or_insert_with(|| RuntimeFailure::Persistence {
                message: error.to_string(),
            })
            .clone();
        state.active = None;
        state.restored = false;
        state.failure.send_replace(Some(failure.clone()));
        state.events.take();
        // Drivers own abort-on-drop tasks. Do not await cleanup while holding session locks.
        state.agent.take();
        self.updates_task.lock().expect("update task lock").take();
        failure
    }

    async fn finish_locked(&self, result: PromptResult) -> Result<(), SessionError> {
        let active = self.lock().active;
        if let Some(turn) = active {
            self.publish_locked([EventKind::TurnFinished {
                turn,
                result: result.clone(),
            }])
            .await?;
            self.complete_locked(CompletionOutcome::Native(result));
        }
        Ok(())
    }

    async fn interrupt_locked(
        &self,
        reason: InterruptionReason,
        terminal: Option<EventKind>,
    ) -> Result<(), SessionError> {
        let active = self.lock().active;
        let mut events = Vec::new();
        if let Some(turn) = active {
            events.push(EventKind::TurnInterrupted {
                turn,
                reason: reason.clone(),
            });
        }
        events.extend(terminal);
        self.publish_locked(events).await?;
        if active.is_some() {
            self.complete_locked(CompletionOutcome::Interrupted(reason));
        }
        Ok(())
    }

    fn complete_locked(&self, outcome: CompletionOutcome) {
        let mut state = self.lock();
        state.active = None;
        if let Some(completions) = &state.completions
            && state.foreground_viewers == 0
        {
            let completion = Completion {
                session: state.meta.clone(),
                outcome,
            };
            if let Err(error) = completions.try_send(completion) {
                tracing::warn!(%error, session = %state.meta.id, "completion notification was not queued");
            }
        }
    }

    async fn fail_locked(&self, message: String) -> Result<(), SessionError> {
        if self.lock().terminal() {
            return Ok(());
        }
        self.interrupt_locked(
            InterruptionReason::BackendFailure {
                message: message.clone(),
            },
            Some(EventKind::Failed { message }),
        )
        .await?;
        let mut state = self.lock();
        state.events.take();
        state.agent.take();
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("session state lock is not poisoned")
    }
}

impl State {
    fn check_health(&self) -> Result<(), SessionError> {
        match &self.runtime.failure {
            Some(failure) => Err(failure.clone().into()),
            None => Ok(()),
        }
    }

    fn terminal(&self) -> bool {
        self.runtime.failure.is_some()
            || matches!(
                self.meta.status,
                SessionStatus::Stopped | SessionStatus::Failed
            )
    }

    fn require_ready(&self) -> Result<(), SessionError> {
        self.check_health()?;
        if self.deleted
            || self.closing
            || self.reloading
            || !self.restored
            || self.agent.is_none()
            || !matches!(
                self.meta.status,
                SessionStatus::Idle | SessionStatus::Working
            )
        {
            return Err(SessionError::Inactive(self.meta.status));
        }
        Ok(())
    }

    fn check_cursor(&self, requested: Option<Cursor>) -> Result<(), HistoryError> {
        if let Some(requested) = requested
            && requested > self.cursor
        {
            return Err(HistoryError::Future {
                requested,
                latest: self.cursor,
            });
        }
        Ok(())
    }
}
