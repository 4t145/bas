use std::{
    future::Future,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

use basr::acp;
use basr::model::{
    Cursor, Event, EventKind, SessionId, SessionMeta, SessionStatus, StopReason, TurnId,
    TurnReceipt, TurnResult,
};
use tokio::sync::{broadcast, mpsc};
use tokio_util::task::AbortOnDropHandle;

#[cfg(test)]
use super::Page;
use super::{AttachOptions, Attachment, HistoryError, Limits, history::Ring};
use crate::agent::{self, Agent, TurnUpdate, Update};
use crate::storage::{self, Storage};

/// A detached snapshot; notification delivery never retains the session.
#[derive(Clone, Debug)]
pub struct Completion {
    pub session: SessionMeta,
    pub result: TurnResult,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("a turn is already running")]
    Busy,
    #[error("session is archived")]
    Archived,
    #[error("session is {0:?}")]
    Inactive(SessionStatus),
    #[error("a prompt must contain at least one content block")]
    EmptyPrompt,
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
    pub(super) restore_gate: tokio::sync::Mutex<()>,
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
    restored: bool,
    deleted: bool,
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
        Arc::new(Self {
            state: Mutex::new(State {
                meta,
                history: Ring::new(limits.history_capacity),
                cursor,
                active: None,
                agent: None,
                events: Some(events),
                completions,
                foreground_viewers: 0,
                startup_config: None,
                startup_mode: None,
                storage,
                restored,
                deleted: false,
            }),
            transaction: tokio::sync::Mutex::new(()),
            updates_task: Mutex::new(None),
            restore_gate: tokio::sync::Mutex::new(()),
        })
    }

    pub(super) fn is_restored(&self) -> bool {
        self.lock().restored
    }

    pub(super) fn restore_agent(&self, agent: Box<dyn Agent>) {
        let mut state = self.lock();
        state.agent = Some(agent);
        state.restored = true;
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

    /// Changes only archival metadata, including for starting or terminal sessions.
    /// Repeating the current value does not publish an event or advance revision.
    #[cfg(test)]
    pub async fn set_archived(self: &Arc<Self>, archived: bool) -> SessionMeta {
        self.try_set_archived(archived)
            .await
            .expect("archival metadata must be committed")
    }

    pub async fn try_set_archived(
        self: &Arc<Self>,
        archived: bool,
    ) -> Result<SessionMeta, SessionError> {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            let changed = {
                let state = session.lock();
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
            Ok(session.meta())
        })
        .await
    }

    pub(super) fn listen(self: &Arc<Self>, mut inbox: mpsc::Receiver<Update>) {
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
    }

    /// # Errors
    /// Returns an error if initialization already failed or shutdown started.
    pub(super) async fn ready(self: &Arc<Self>, agent: Box<dyn Agent>) -> Result<(), SessionError> {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            let mut meta = {
                let state = session.lock();
                if state.terminal() {
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
        let (storage, id, cursor, events) = {
            let state = self.lock();
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
                ));
            }
            (
                state.storage.clone().expect("storage checked"),
                state.meta.id,
                state.cursor,
                state.events.as_ref().map(broadcast::Sender::subscribe),
            )
        };
        if options.cursor.is_none() && !storage.is_persistent() {
            let backlog = storage.tail(id, options.replay).await?;
            return Ok(Attachment::new(backlog, Cursor::START, events));
        }
        let after = options.cursor.unwrap_or_else(|| {
            Cursor::from_raw(cursor.raw().saturating_sub(options.replay as u64))
        });
        if options.cursor.is_some() {
            storage.page(id, Some(after), 1).await?;
        }
        Ok(Attachment::persistent(after, cursor, storage, id, events))
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
    /// Rejects empty prompts, overlapping turns, archived/inactive sessions and unsupported content.
    #[cfg(test)]
    pub async fn prompt(
        self: &Arc<Self>,
        content: Vec<acp::ContentBlock>,
    ) -> Result<TurnId, SessionError> {
        self.accept_prompt(acp::PromptRequest::new(self.id().to_string(), content))
            .await
            .map(|receipt| receipt.turn)
    }

    /// Accepts a turn and returns the cursor of its TurnStarted event under the same lock.
    ///
    /// # Errors
    /// Rejects empty prompts, overlapping turns, archived/inactive sessions and backend refusals.
    pub async fn accept_prompt(
        self: &Arc<Self>,
        request: acp::PromptRequest,
    ) -> Result<TurnReceipt, SessionError> {
        self.owned(move |session| async move {
            let _gate = session.transaction.lock().await;
            let started_at = {
                let state = session.lock();
                if state.meta.archived {
                    return Err(SessionError::Archived);
                }
                state.require_ready()?;
                if state.active.is_some() {
                    return Err(SessionError::Busy);
                }
                if request.prompt.is_empty() {
                    return Err(SessionError::EmptyPrompt);
                }
                state.cursor.next()
            };
            let turn = TurnId::new();
            let mut events = vec![EventKind::TurnStarted { turn }];
            events.extend(request.prompt.iter().cloned().map(|content| {
                EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
                    acp::ContentChunk::new(content).meta(request.meta.clone()),
                ))
            }));
            session.publish_locked(events).await?;
            let result = {
                let mut state = session.lock();
                state.active = Some(turn);
                state
                    .agent
                    .as_mut()
                    .expect("ready backend")
                    .prompt(turn, request)
            };
            if let Err(error) = result {
                if matches!(error, agent::Error::Backend { .. }) {
                    session.fail_locked(error.to_string()).await?;
                } else {
                    session.finish_locked(Err(error.to_string())).await?;
                }
                return Err(error.into());
            }
            Ok(TurnReceipt {
                turn,
                cursor: started_at,
            })
        })
        .await
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
        .await?;
        Ok(response)
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
        .await?;
        Ok(response)
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

    /// Closes the event stream before awaiting backend cleanup. Safe to call repeatedly.
    pub async fn shutdown(self: &Arc<Self>) {
        let session = Arc::clone(self);
        self.owned(move |_| async move { session.shutdown_owned().await })
            .await;
    }

    async fn shutdown_owned(&self) {
        let _gate = self.transaction.lock().await;
        if !self.lock().terminal() {
            let finished = match self
                .finish_locked(Ok(acp::PromptResponse::new(StopReason::Cancelled)))
                .await
            {
                Ok(()) => true,
                Err(error) => {
                    tracing::error!(%error, "could not commit turn completion during shutdown");
                    false
                }
            };
            if finished
                && let Err(error) = self
                    .publish_locked([EventKind::StatusChanged {
                        status: SessionStatus::Stopped,
                    }])
                    .await
            {
                tracing::error!(%error, "could not commit session shutdown status");
            }
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
                Update::Commit { update, committed } => {
                    let result = session.publish_locked([EventKind::Acp(update)]).await;
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
                    }
                }
                Update::Turn { .. } => Ok(()),
                Update::Barrier(_) => Ok(()),
            };
            if let Err(error) = result {
                let mut state = session.lock();
                tracing::error!(session = %state.meta.id, %error, "session storage commit failed");
                state.events.take();
                state.agent.take();
                state.meta.status = SessionStatus::Failed;
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
        let (events, storage) = {
            let state = self.lock();
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
        if events.is_empty() {
            return Ok(());
        }
        if let Some(storage) = storage {
            storage.commit(meta.clone(), &events).await?;
        }
        let mut state = self.lock();
        state.meta = meta.clone();
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

    async fn finish_locked(&self, result: TurnResult) -> Result<(), SessionError> {
        let active = self.lock().active;
        if let Some(turn) = active {
            self.publish_locked([EventKind::TurnFinished {
                turn,
                result: result.clone(),
            }])
            .await?;
            let mut state = self.lock();
            state.active = None;
            if let Some(completions) = &state.completions
                && state.foreground_viewers == 0
            {
                let completion = Completion {
                    session: state.meta.clone(),
                    result,
                };
                if let Err(error) = completions.try_send(completion) {
                    tracing::warn!(%error, session = %state.meta.id, "completion notification was not queued");
                }
            }
        }
        Ok(())
    }

    async fn fail_locked(&self, message: String) -> Result<(), SessionError> {
        if self.lock().terminal() {
            return Ok(());
        }
        self.finish_locked(Err(message.clone())).await?;
        self.publish_locked([EventKind::Failed { message }]).await?;
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
    fn terminal(&self) -> bool {
        matches!(
            self.meta.status,
            SessionStatus::Stopped | SessionStatus::Failed
        )
    }

    fn require_ready(&self) -> Result<(), SessionError> {
        if self.deleted
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
