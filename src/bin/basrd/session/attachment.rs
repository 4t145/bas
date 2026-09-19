use std::collections::VecDeque;

use basr::{
    model::{Cursor, Event},
    service::DEFAULT_REPLAY,
};
use futures::{Stream, stream};
use tokio::sync::broadcast;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttachOptions {
    /// Recent events when cursor is absent. Cursor resumption replays the entire window after it.
    pub replay: usize,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AttachError {
    #[error("client is {0} events behind; re-attach from the last cursor")]
    Lagged(u64),
    #[error("the session ended")]
    Closed,
}

/// Owns only a history snapshot and receiver; detaching never owns or stops the session.
pub struct Attachment {
    backlog: VecDeque<Event>,
    cursor: Cursor,
    events: Option<broadcast::Receiver<Event>>,
}

impl Attachment {
    pub(super) fn new(
        backlog: Vec<Event>,
        cursor: Cursor,
        events: Option<broadcast::Receiver<Event>>,
    ) -> Self {
        Self {
            backlog: backlog.into(),
            cursor,
            events,
        }
    }

    #[cfg(test)]
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Reads retained history first, then events published after the snapshot.
    ///
    /// # Errors
    /// Returns an error when the stream ends or a slow reader loses live events.
    pub async fn recv(&mut self) -> Result<Event, AttachError> {
        let event = if let Some(event) = self.backlog.pop_front() {
            event
        } else {
            let events = self.events.as_mut().ok_or(AttachError::Closed)?;
            match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    return Err(AttachError::Lagged(skipped));
                }
                Err(broadcast::error::RecvError::Closed) => return Err(AttachError::Closed),
            }
        };
        debug_assert!(
            event.cursor > self.cursor,
            "session events must arrive in cursor order"
        );
        self.cursor = event.cursor;
        Ok(event)
    }

    /// Dropping the stream releases the attachment without a forwarding task.
    pub fn into_stream(self) -> impl Stream<Item = Result<Event, AttachError>> + Send + 'static {
        stream::unfold(Some(self), |attachment| async move {
            let mut attachment = attachment?;
            match attachment.recv().await {
                Ok(event) => Some((Ok(event), Some(attachment))),
                Err(AttachError::Closed) => None,
                Err(error) => Some((Err(error), None)),
            }
        })
    }
}
