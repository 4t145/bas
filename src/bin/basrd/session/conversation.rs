use std::sync::{Arc, Mutex, MutexGuard, Weak};

use basr::acp;
use basr::model::{
    Cursor, Event, EventKind, SessionId, SessionMeta, SessionStatus, StopReason, TurnId, TurnResult,
};
use tokio::sync::{broadcast, mpsc};
use tokio_util::task::AbortOnDropHandle;

use super::{AttachOptions, Attachment, HistoryError, Limits, Page, history::Ring};
use crate::agent::{self, Agent, TurnUpdate, Update};

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
    #[error("session is {0:?}")]
    Inactive(SessionStatus),
    #[error("a prompt must contain at least one content block")]
    EmptyPrompt,
    #[error(transparent)]
    Agent(#[from] agent::Error),
}

pub struct Session {
    state: Mutex<State>,
    updates_task: Mutex<Option<AbortOnDropHandle<()>>>,
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
}

impl Session {
    pub(super) fn new(
        meta: SessionMeta,
        limits: Limits,
        completions: Option<mpsc::Sender<Completion>>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(limits.live_capacity.get());
        Arc::new(Self {
            state: Mutex::new(State {
                meta,
                history: Ring::new(limits.history_capacity),
                cursor: Cursor::START,
                active: None,
                agent: None,
                events: Some(events),
                completions,
                foreground_viewers: 0,
            }),
            updates_task: Mutex::new(None),
        })
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

    pub(super) fn listen(self: &Arc<Self>, mut inbox: mpsc::Receiver<Update>) {
        let weak = Arc::downgrade(self);
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            while let Some(update) = inbox.recv().await {
                let Some(session) = weak.upgrade() else {
                    return;
                };
                session.apply(update);
                if session.lock().terminal() {
                    return;
                }
            }
            if let Some(session) = weak.upgrade() {
                session.fail("backend update channel closed".to_owned());
            }
        }));
        *self
            .updates_task
            .lock()
            .expect("update task lock is not poisoned") = Some(task);
    }

    /// # Errors
    /// Returns an error if initialization already failed or shutdown started.
    pub(super) fn ready(&self, agent: Box<dyn Agent>) -> Result<(), SessionError> {
        let mut state = self.lock();
        if state.terminal() {
            return Err(SessionError::Inactive(state.meta.status));
        }
        state.meta.initialization = agent.initialization();
        state.meta.acp = agent.session_info().map(|mut info| {
            info.session_id = state.meta.id.to_string().into();
            info
        });
        // Notifications may arrive during session/new, before launch returns.
        for event in state
            .history
            .page(None, usize::MAX)
            .expect("nonzero history page")
            .events
        {
            if let EventKind::Acp(update) = event.kind {
                state.meta.apply_acp(&update);
            }
        }
        state.agent = Some(agent);
        state.publish(EventKind::StatusChanged {
            status: SessionStatus::Idle,
        });
        Ok(())
    }

    /// Snapshots history and subscribes atomically with respect to publication.
    ///
    /// # Errors
    /// Rejects expired or future cursors instead of silently skipping history.
    pub fn attach(&self, options: AttachOptions) -> Result<Attachment, HistoryError> {
        let state = self.lock();
        state.check_cursor(options.cursor)?;
        let backlog = match options.cursor {
            Some(cursor) => state.history.page(Some(cursor), usize::MAX)?.events,
            None => state.history.tail(options.replay),
        };
        Ok(Attachment::new(
            backlog,
            options.cursor.unwrap_or(Cursor::START),
            state.events.as_ref().map(broadcast::Sender::subscribe),
        ))
    }

    /// # Errors
    /// Rejects expired/future cursors and a zero page size.
    pub fn page(&self, after: Option<Cursor>, limit: usize) -> Result<Page, HistoryError> {
        let state = self.lock();
        state.check_cursor(after)?;
        state.history.page(after, limit)
    }

    /// Accepts exactly one turn and commits the user message before its updates.
    ///
    /// # Errors
    /// Rejects empty prompts, overlapping turns, inactive sessions and unsupported content.
    #[cfg(test)]
    pub fn prompt(&self, content: Vec<acp::ContentBlock>) -> Result<TurnId, SessionError> {
        self.prompt_acp(acp::PromptRequest::new(self.id().to_string(), content))
    }

    /// # Errors
    /// Rejects empty prompts, overlapping turns and inactive sessions.
    pub fn prompt_acp(&self, request: acp::PromptRequest) -> Result<TurnId, SessionError> {
        let mut state = self.lock();
        state.require_ready()?;
        if state.active.is_some() {
            return Err(SessionError::Busy);
        }
        if request.prompt.is_empty() {
            return Err(SessionError::EmptyPrompt);
        }
        let turn = TurnId::new();
        if let Err(error) = state
            .agent
            .as_mut()
            .expect("a ready session owns a backend")
            .prompt(turn, request.clone())
        {
            if matches!(error, agent::Error::Backend { .. }) {
                state.fail(error.to_string());
            }
            return Err(error.into());
        }
        state.active = Some(turn);
        state.publish(EventKind::TurnStarted { turn });
        for content in request.prompt {
            state.publish(EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
                acp::ContentChunk::new(content).meta(request.meta.clone()),
            )));
        }
        Ok(turn)
    }

    /// # Errors
    /// Rejects inactive sessions or reports a backend cancellation failure.
    #[cfg(test)]
    pub fn cancel(&self) -> Result<(), SessionError> {
        self.cancel_acp(acp::CancelNotification::new(self.id().to_string()))
    }

    /// # Errors
    /// Rejects inactive sessions or reports a backend cancellation failure.
    pub fn cancel_acp(&self, request: acp::CancelNotification) -> Result<(), SessionError> {
        let mut state = self.lock();
        state.require_ready()?;
        if state.active.is_none() {
            return Ok(());
        }
        if let Err(error) = state
            .agent
            .as_mut()
            .expect("a ready session owns a backend")
            .cancel(request)
        {
            if matches!(error, agent::Error::Backend { .. }) {
                state.fail(error.to_string());
            }
            return Err(error.into());
        }
        Ok(())
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
        let mut state = self.lock();
        state.require_ready()?;
        state.publish(EventKind::Acp(acp::SessionUpdate::ConfigOptionUpdate(
            acp::ConfigOptionUpdate::new(response.config_options.clone())
                .meta(response.meta.clone()),
        )));
        Ok(response)
    }

    /// # Errors
    /// Returns backend rejection or an inactive session error.
    pub async fn set_mode(
        &self,
        request: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, SessionError> {
        let mode = request.mode_id.clone();
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
        let mut state = self.lock();
        state.require_ready()?;
        state.publish(EventKind::Acp(acp::SessionUpdate::CurrentModeUpdate(
            acp::CurrentModeUpdate::new(mode),
        )));
        Ok(response)
    }

    /// Titles are daemon metadata, including for backends without a rename operation.
    ///
    /// # Errors
    /// Rejects inactive sessions.
    pub fn set_title(&self, title: &str) -> Result<(), SessionError> {
        let mut state = self.lock();
        state.require_ready()?;
        state.rename(title.to_owned());
        Ok(())
    }

    /// Closes the event stream before awaiting backend cleanup. Safe to call repeatedly.
    pub async fn shutdown(&self) {
        let agent = {
            let mut state = self.lock();
            if !state.terminal() {
                state.finish(Ok(acp::PromptResponse::new(StopReason::Cancelled)));
                state.publish(EventKind::StatusChanged {
                    status: SessionStatus::Stopped,
                });
                state.events.take();
            }
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

    pub(super) fn apply(&self, update: Update) {
        let mut state = self.lock();
        if state.terminal() {
            return;
        }
        match update {
            Update::Acp(update) => state.publish(EventKind::Acp(update)),
            Update::Failed(message) => state.fail(message),
            Update::Turn { turn, update } if state.active == Some(turn) => match update {
                TurnUpdate::Acp(update) => state.publish(EventKind::Acp(update)),
                TurnUpdate::Finished(result) => state.finish(result),
            },
            Update::Turn { .. } => {} // Late updates cannot mutate another turn or revive a session.
        }
    }

    pub(super) fn fail(&self, message: String) {
        self.lock().fail(message);
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
        if !matches!(
            self.meta.status,
            SessionStatus::Idle | SessionStatus::Working
        ) {
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

    fn rename(&mut self, title: String) {
        if self.meta.title.as_ref() != Some(&title) {
            self.publish(EventKind::TitleChanged { title });
        }
    }

    fn finish(&mut self, result: TurnResult) {
        if let Some(turn) = self.active.take() {
            self.publish(EventKind::TurnFinished {
                turn,
                result: result.clone(),
            });
            if let Some(completions) = &self.completions
                && self.foreground_viewers == 0
            {
                let completion = Completion {
                    session: self.meta.clone(),
                    result,
                };
                if let Err(error) = completions.try_send(completion) {
                    tracing::warn!(%error, session = %self.meta.id, "completion notification was not queued");
                }
            }
        }
    }

    fn fail(&mut self, message: String) {
        if self.terminal() {
            return;
        }
        self.finish(Err(message.clone()));
        self.publish(EventKind::Failed { message });
        self.events.take();
        self.agent.take();
    }

    fn publish(&mut self, kind: EventKind) {
        self.cursor = self.cursor.next();
        let event = Event::new(self.meta.id, self.cursor, kind);
        match &event.kind {
            EventKind::Acp(update) => self.meta.apply_acp(update),
            EventKind::TitleChanged { title } => self.meta.title = Some(title.clone()),
            EventKind::StatusChanged { status } => self.meta.status = *status,
            EventKind::TurnStarted { .. } => self.meta.status = SessionStatus::Working,
            EventKind::TurnFinished { .. } => self.meta.status = SessionStatus::Idle,
            EventKind::Failed { .. } => self.meta.status = SessionStatus::Failed,
        }
        self.meta.updated_at = event.at;
        self.history.push(event.clone());
        if let Some(events) = &self.events {
            let _ = events.send(event);
        }
    }
}
