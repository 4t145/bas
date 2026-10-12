//! Durable session snapshots and ordered event history, independent of the daemon runtime.
//!
//! A commit replaces the metadata snapshot and appends all its events atomically.
//! Callers must publish events only after `commit` returns successfully.

#[path = "storage/localdb.rs"]
mod localdb;
#[path = "storage/memory.rs"]
mod memory;

pub use localdb::LocalDb;
pub use memory::Memory;

use basr::model::{Cursor, Event, SessionId, SessionMeta, TurnId, TurnReceipt};
use futures::future::BoxFuture;

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;

/// Private backend state. Version and bytes are deliberately opaque to storage and clients.
/// In version 1, ACP session ids can be stored as their UTF-8 bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeData {
    pub version: u32,
    pub payload: Vec<u8>,
}

impl ResumeData {
    #[must_use]
    pub fn acp(session_id: impl AsRef<str>) -> Self {
        Self {
            version: 1,
            payload: session_id.as_ref().as_bytes().to_vec(),
        }
    }

    /// # Errors
    /// Returns an error for unsupported versions or invalid UTF-8.
    pub fn acp_session_id(&self) -> Result<&str, Error> {
        if self.version != 1 {
            return Err(Error::UnsupportedResumeVersion(self.version));
        }
        Ok(std::str::from_utf8(&self.payload)?)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredSession {
    pub meta: SessionMeta,
    pub resume: Option<ResumeData>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub events: Vec<Event>,
    /// Cursor to pass as `after` for the next page; `None` means exhausted.
    pub next: Option<Cursor>,
}

/// Small acceptance ledger entry retained until its session is deleted,
/// independently of history eviction. It never stores a second prompt payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptAcceptance {
    pub receipt: TurnReceipt,
    pub fingerprint: [u8; 32],
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("session {0} does not exist")]
    NotFound(SessionId),
    #[error("session {0} already exists")]
    AlreadyExists(SessionId),
    #[error("invalid commit: {0}")]
    InvalidCommit(&'static str),
    #[error("submission {0} is already registered")]
    SubmissionExists(TurnId),
    #[error("history page limit must be greater than zero")]
    EmptyPage,
    #[error("cursor {requested} is ahead of latest {latest}")]
    Future { requested: Cursor, latest: Cursor },
    #[error("history after {requested} was evicted through {evicted_through}")]
    Expired {
        requested: Cursor,
        evicted_through: Cursor,
    },
    #[error("cursor is too large for SQLite")]
    CursorOverflow,
    #[error("stored session data is inconsistent: {0}")]
    Corrupt(&'static str),
    #[error("SQLite worker stopped before completing the operation")]
    WorkerClosed,
    #[error("unsupported resume data version {0}")]
    UnsupportedResumeVersion(u32),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    Database(#[from] toasty::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Writes must finish before the daemon acknowledges or broadcasts.
pub trait Storage: Send + Sync {
    /// # Errors
    /// Fails for a duplicate id, invalid initial metadata, or backend I/O errors.
    fn create(
        &self,
        meta: SessionMeta,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), Error>>;
    /// Commits a metadata-only edit or one or more contiguous new events.
    ///
    /// `meta.revision` must match the final event's cursor (or the previous revision
    /// for metadata-only edits). Any error leaves the prior snapshot and history intact.
    ///
    /// # Errors
    /// Fails if the session is missing, events are not contiguous, metadata is
    /// inconsistent, or the backend cannot commit.
    fn commit<'a>(
        &'a self,
        meta: SessionMeta,
        events: &'a [Event],
        acceptance: Option<PromptAcceptance>,
    ) -> BoxFuture<'a, Result<(), Error>>;
    /// Reads a registered submission without changing history or starting a backend.
    ///
    /// # Errors
    /// Fails for a missing session, malformed records or storage I/O failures.
    fn acceptance(
        &self,
        session: SessionId,
        turn: TurnId,
    ) -> BoxFuture<'_, Result<Option<PromptAcceptance>, Error>>;
    /// Replaces private backend data when it becomes available after session creation.
    ///
    /// # Errors
    /// Fails for a missing session or backend I/O errors.
    fn save_resume(
        &self,
        id: SessionId,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), Error>>;
    /// # Errors
    /// Fails on corrupt persisted data or backend I/O errors.
    fn load(&self, id: SessionId) -> BoxFuture<'_, Result<Option<StoredSession>, Error>>;
    /// Lists all sessions including archived ones, newest first.
    ///
    /// # Errors
    /// Fails on corrupt persisted data or backend I/O errors.
    fn list(&self) -> BoxFuture<'_, Result<Vec<SessionMeta>, Error>>;
    /// Reads events strictly after `after` in ascending cursor order.
    /// `None` starts from the oldest *retained* event; `Some(Cursor::START)`
    /// requests the entire history and can therefore expire in memory.
    ///
    /// # Errors
    /// Fails for a missing session, zero page size, future/expired cursor,
    /// corrupt persisted data, or backend I/O errors.
    fn page(
        &self,
        id: SessionId,
        after: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Page, Error>>;
    /// Returns the most recent `count` events in ascending cursor order.
    ///
    /// # Errors
    /// Fails for a missing session, corrupt persisted data, or backend I/O errors.
    fn tail(&self, id: SessionId, count: usize) -> BoxFuture<'_, Result<Vec<Event>, Error>>;
    /// # Errors
    /// Fails on backend I/O errors; missing sessions return `false`.
    fn delete(&self, id: SessionId) -> BoxFuture<'_, Result<bool, Error>>;
    fn is_persistent(&self) -> bool;
}

fn validate_create(meta: &SessionMeta) -> Result<(), Error> {
    if meta.revision != Cursor::START {
        return Err(Error::InvalidCommit(
            "new session must have starting revision",
        ));
    }
    Ok(())
}

fn validate_commit(
    previous: &SessionMeta,
    meta: &SessionMeta,
    events: &[Event],
) -> Result<(), Error> {
    if previous.id != meta.id {
        return Err(Error::InvalidCommit("session id changed"));
    }
    if meta.revision.raw() > i64::MAX as u64 {
        return Err(Error::CursorOverflow);
    }
    let mut cursor = previous.revision.raw();
    for event in events {
        cursor = cursor
            .checked_add(1)
            .ok_or(Error::InvalidCommit("cursor overflow"))?;
        if event.session != meta.id || event.cursor.raw() != cursor {
            return Err(Error::InvalidCommit(
                "events must match session and be contiguous",
            ));
        }
    }
    if meta.revision.raw() != cursor {
        return Err(Error::InvalidCommit(
            "metadata revision must match last event",
        ));
    }
    Ok(())
}

fn check_page(after: Option<Cursor>, limit: usize, latest: Cursor) -> Result<(), Error> {
    if limit == 0 {
        return Err(Error::EmptyPage);
    }
    if let Some(requested) = after
        && requested > latest
    {
        return Err(Error::Future { requested, latest });
    }
    Ok(())
}

fn validate_acceptance(events: &[Event], acceptance: PromptAcceptance) -> Result<(), Error> {
    if acceptance.receipt.cursor == Cursor::START
        || !events.iter().any(|event| {
            event.cursor == acceptance.receipt.cursor
                && matches!(event.kind, basr::model::EventKind::TurnStarted { turn } if turn == acceptance.receipt.turn)
        })
    {
        return Err(Error::InvalidCommit("acceptance requires its TurnStarted event"));
    }
    Ok(())
}
