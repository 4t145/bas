use std::collections::VecDeque;
use std::sync::Arc;

use crate::storage::Storage;
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

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AttachError {
    #[error("client is {0} events behind; re-attach from the last cursor")]
    Lagged(u64),
    #[error("the session ended")]
    Closed,
    #[error("cannot read persisted history: {0}")]
    Storage(String),
}

/// Owns only a history snapshot and receiver; detaching never owns or stops the session.
pub struct Attachment {
    backlog: VecDeque<Event>,
    cursor: Cursor,
    events: Option<broadcast::Receiver<Event>>,
    persistent: Option<(Arc<dyn Storage>, basr::model::SessionId, Cursor)>,
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
            persistent: None,
        }
    }

    pub(super) fn persistent(
        cursor: Cursor,
        snapshot: Cursor,
        storage: Arc<dyn Storage>,
        session: basr::model::SessionId,
        events: Option<broadcast::Receiver<Event>>,
    ) -> Self {
        let mut attachment = Self::new(Vec::new(), cursor, events);
        attachment.persistent = Some((storage, session, snapshot));
        attachment
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
        loop {
            if self.backlog.is_empty()
                && let Some((storage, session, snapshot)) = &self.persistent
                && self.cursor < *snapshot
            {
                let page = storage
                    .page(*session, Some(self.cursor), 256)
                    .await
                    .map_err(|error| match error {
                        crate::storage::Error::Expired { .. } => {
                            AttachError::Lagged(snapshot.raw().saturating_sub(self.cursor.raw()))
                        }
                        other => AttachError::Storage(other.to_string()),
                    })?;
                self.backlog.extend(
                    page.events
                        .into_iter()
                        .take_while(|event| event.cursor <= *snapshot),
                );
                if self.backlog.is_empty() {
                    return Err(AttachError::Storage(
                        "stored history has a gap before its committed cursor".into(),
                    ));
                }
            }
            if let Some(event) = self.backlog.pop_front() {
                debug_assert!(
                    event.cursor > self.cursor,
                    "session events must arrive in cursor order"
                );
                self.cursor = event.cursor;
                return Ok(event);
            }
            let events = self.events.as_mut().ok_or(AttachError::Closed)?;
            match events.recv().await {
                Ok(event) if event.is_transient() => return Ok(event),
                Ok(event) if event.cursor <= self.cursor => {}
                Ok(event) if self.persistent.is_some() && event.cursor > self.cursor.next() => {
                    if let Some((_, _, snapshot)) = &mut self.persistent {
                        *snapshot = event.cursor;
                    }
                }
                Ok(event) => {
                    self.cursor = event.cursor;
                    return Ok(event);
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    let Some((storage, session, snapshot)) = &mut self.persistent else {
                        return Err(AttachError::Lagged(skipped));
                    };
                    let latest = storage
                        .load(*session)
                        .await
                        .map_err(|error| AttachError::Storage(error.to_string()))?
                        .ok_or(AttachError::Closed)?
                        .meta
                        .revision;
                    *snapshot = latest;
                }
                Err(broadcast::error::RecvError::Closed) => return Err(AttachError::Closed),
            }
        }
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
