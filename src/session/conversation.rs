use std::sync::{Mutex, RwLock};

use tokio::sync::broadcast;

use crate::agent::{Agent, Error as AgentError};
use crate::model::{
    ContentBlock, Cursor, Event, EventKind, Message, Role, SessionId, SessionMeta, SessionStatus,
    TurnId,
};

use super::{AttachOptions, Attachment, Page, Ring};

/// Number of events a session buffers for clients that are momentarily slow.
/// A client that falls further behind is told to re-attach from its cursor.
const BROADCAST_CAPACITY: usize = 1024;

/// One live conversation.
///
/// The session owns the backend connection and the replay window. It stays
/// alive while frontends detach: only the backend finishing or an explicit
/// [`Registry::remove`](super::Registry::remove) ends it.
pub struct Session {
    meta: RwLock<SessionMeta>,
    state: Mutex<State>,
    events: broadcast::Sender<Event>,
    agent: Box<dyn Agent>,
}

struct State {
    history: Ring,
    next: Cursor,
}

impl Session {
    pub(crate) fn new(meta: SessionMeta, agent: Box<dyn Agent>, history_capacity: usize) -> Self {
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);

        Self {
            meta: RwLock::new(meta),
            state: Mutex::new(State {
                history: Ring::new(history_capacity),
                // Cursors are 1-based so that `Cursor::START` can mean "before
                // the first event" without colliding with a real cursor.
                next: Cursor::START.next(),
            }),
            events,
            agent,
        }
    }

    /// Identifier of this session.
    #[must_use]
    pub fn id(&self) -> SessionId {
        self.read_meta().id
    }

    /// A snapshot of the session's metadata.
    #[must_use]
    pub fn meta(&self) -> SessionMeta {
        self.read_meta().clone()
    }

    /// Attaches a client, replaying history as requested.
    ///
    /// The broadcast subscription is created before the backlog is read so no
    /// live event can slip between the two; duplicates are dropped by cursor.
    #[must_use]
    pub fn attach(&self, options: AttachOptions) -> Attachment {
        let events = self.events.subscribe();

        let backlog = {
            let state = self
                .state
                .lock()
                .expect("the session state mutex is never poisoned");
            match options.cursor {
                Some(cursor) => state.history.page(cursor, options.replay).events,
                None => state.history.tail(options.replay),
            }
        };

        Attachment::new(self.id(), backlog, events)
    }

    /// Loads a page of replayable events strictly after `after`.
    #[must_use]
    pub fn page(&self, after: Cursor, limit: usize) -> Page {
        self.state
            .lock()
            .expect("the session state mutex is never poisoned")
            .history
            .page(after, limit)
    }

    /// Returns the newest `count` committed messages, oldest first.
    #[must_use]
    pub fn recent_messages(&self, count: usize) -> Vec<Event> {
        self.state
            .lock()
            .expect("the session state mutex is never poisoned")
            .history
            .messages(count)
    }

    /// Number of replayable events currently kept.
    #[must_use]
    pub fn history_len(&self) -> usize {
        self.state
            .lock()
            .expect("the session state mutex is never poisoned")
            .history
            .len()
    }

    /// Sends a user turn to the backend.
    ///
    /// The user message is committed to the timeline before the backend is
    /// asked, so a client that attaches mid-turn sees the request even when the
    /// backend then fails.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] when the backend rejects or cannot accept the
    /// turn.
    pub async fn prompt(&self, content: Vec<ContentBlock>) -> Result<TurnId, AgentError> {
        self.publish(EventKind::Message(Message::new(
            Role::User,
            content.clone(),
        )));
        self.agent.prompt(content).await
    }

    /// Asks the backend to abandon the running turn.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] when the backend cannot cancel.
    pub async fn cancel(&self) -> Result<(), AgentError> {
        self.agent.cancel().await
    }

    /// Renames the session on both the daemon and the backend.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] when the backend cannot rename the session. The
    /// daemon-side title is updated only after the backend accepted it.
    pub async fn set_title(&self, title: &str) -> Result<(), AgentError> {
        self.agent.set_title(title).await?;
        self.publish(EventKind::TitleChanged {
            title: title.to_owned(),
        });
        Ok(())
    }

    /// Stops the backend. The session stays readable until it is removed.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] when the backend could not be shut down.
    pub async fn shutdown(&self) -> Result<(), AgentError> {
        self.set_status(SessionStatus::Stopped);
        self.agent.shutdown().await
    }

    /// Commits one backend update to the timeline and broadcasts it.
    pub(crate) fn apply(&self, update: crate::agent::Update) {
        self.publish(update.into());
    }

    /// Publishes a lifecycle transition.
    pub(crate) fn set_status(&self, status: SessionStatus) {
        self.publish(EventKind::StatusChanged { status });
    }

    /// Marks the session failed, for example when the backend dies.
    pub(crate) fn fail(&self) {
        self.set_status(SessionStatus::Failed);
    }

    fn publish(&self, kind: EventKind) -> Event {
        let event = {
            let mut state = self
                .state
                .lock()
                .expect("the session state mutex is never poisoned");
            let cursor = state.next;
            state.next = cursor.next();

            let event = Event::new(self.id(), cursor, kind);
            if event.is_replayable() {
                state.history.push(event.clone());
            }
            event
        };

        self.observe(&event);
        // A session with no attached client is normal: detaching must not stop
        // the agent loop.
        let _ = self.events.send(event.clone());
        event
    }

    /// Folds lifecycle events into the session metadata.
    fn observe(&self, event: &Event) {
        let mut meta = self
            .meta
            .write()
            .expect("the session metadata lock is never poisoned");

        let mut changed = true;
        match &event.kind {
            EventKind::TitleChanged { title } => meta.title = Some(title.clone()),
            EventKind::StatusChanged { status } => meta.status = *status,
            EventKind::TurnStarted { .. } => meta.status = SessionStatus::Working,
            EventKind::TurnFinished { .. } => {
                if meta.status == SessionStatus::Working {
                    meta.status = SessionStatus::Idle;
                } else {
                    changed = false;
                }
            }
            _ => changed = false,
        }

        if changed {
            meta.updated_at = event.at;
        }
    }

    fn read_meta(&self) -> std::sync::RwLockReadGuard<'_, SessionMeta> {
        self.meta
            .read()
            .expect("the session metadata lock is never poisoned")
    }
}
