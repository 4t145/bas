use futures::{Stream, stream};
use tokio::sync::broadcast;

use crate::model::{Cursor, Event, SessionId};

/// Replayable events a fresh attachment receives when the client does not ask
/// for a specific position.
pub const DEFAULT_REPLAY: usize = 32;

/// What an attaching client wants to see first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttachOptions {
    /// Number of recent replayable events to deliver before live events, or the
    /// page size when [`cursor`](Self::cursor) is set.
    pub replay: usize,
    /// Position to resume from. `None` means "the newest `replay` events".
    pub cursor: Option<Cursor>,
}

impl Default for AttachOptions {
    fn default() -> Self {
        Self {
            replay: DEFAULT_REPLAY,
            cursor: None,
        }
    }
}

impl AttachOptions {
    /// Replays the newest `replay` events and then follows the session live.
    #[must_use]
    pub fn last(replay: usize) -> Self {
        Self {
            replay,
            cursor: None,
        }
    }

    /// Resumes strictly after `cursor`, delivering at most `replay` events per
    /// page.
    #[must_use]
    pub fn from_cursor(cursor: Cursor, replay: usize) -> Self {
        Self {
            replay,
            cursor: Some(cursor),
        }
    }
}

/// Why an attachment stopped delivering events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AttachError {
    /// The client fell behind and the session dropped events. Re-attach with
    /// the last cursor the client processed.
    #[error("client is {0} events behind; re-attach from the last cursor")]
    Lagged(u64),
    /// The session ended.
    #[error("the session ended")]
    Closed,
}

/// A client's view of one session.
///
/// Dropping the attachment detaches the client; the agent keeps running.
pub struct Attachment {
    session: SessionId,
    backlog: Vec<Event>,
    delivered: usize,
    cursor: Cursor,
    events: broadcast::Receiver<Event>,
}

impl Attachment {
    pub(crate) fn new(
        session: SessionId,
        backlog: Vec<Event>,
        events: broadcast::Receiver<Event>,
    ) -> Self {
        let cursor = backlog.last().map_or(Cursor::START, |event| event.cursor);
        Self {
            session,
            backlog,
            delivered: 0,
            cursor,
            events,
        }
    }

    /// The session this attachment follows.
    #[must_use]
    pub fn session(&self) -> SessionId {
        self.session
    }

    /// Events the client has not consumed yet.
    #[must_use]
    pub fn backlog(&self) -> &[Event] {
        &self.backlog[self.delivered..]
    }

    /// Cursor of the last event delivered to the client.
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Returns the next event: replayed history first, then live events.
    ///
    /// Events already delivered are skipped, so a client that subscribes before
    /// reading history sees every event exactly once.
    ///
    /// # Errors
    ///
    /// Returns [`AttachError`] when the client fell behind or the session
    /// ended.
    pub async fn recv(&mut self) -> Result<Event, AttachError> {
        if self.delivered < self.backlog.len() {
            let event = self.backlog[self.delivered].clone();
            self.delivered += 1;
            return Ok(event);
        }

        loop {
            match self.events.recv().await {
                Ok(event) if event.cursor > self.cursor => {
                    self.cursor = event.cursor;
                    return Ok(event);
                }
                // Already handed to the client through the backlog.
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    return Err(AttachError::Lagged(skipped));
                }
                Err(broadcast::error::RecvError::Closed) => return Err(AttachError::Closed),
            }
        }
    }

    /// Turns the attachment into a stream, backlog first.
    ///
    /// The stream ends after the first error: a client that lagged must
    /// re-attach with its last cursor to recover the missed events.
    pub fn into_stream(self) -> impl Stream<Item = Result<Event, AttachError>> + Send + 'static {
        stream::unfold(Some(self), |attachment| async move {
            let mut attachment = attachment?;
            match attachment.recv().await {
                Ok(event) => Some((Ok(event), Some(attachment))),
                Err(error) => Some((Err(error), None)),
            }
        })
    }
}
