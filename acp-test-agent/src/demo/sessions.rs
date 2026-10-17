//! In-memory session storage shared by both protocol implementations.
//!
//! The store is intentionally generic over the protocol's `SessionUpdate` type:
//! each connection records exactly the typed updates it sent, so `session/load`
//! and `session/resume` can replay them without a lossy neutral format.

use std::path::{Path, PathBuf};

use thiserror::Error;

use super::SessionConfig;

#[cfg(test)]
mod tests;

/// Errors produced by session bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// The requested session does not exist.
    #[error("unknown session `{0}`")]
    UnknownSession(String),
    /// The session was closed and cannot accept work.
    #[error("session `{0}` is closed")]
    ClosedSession(String),
    /// A prompt is already running for this session.
    #[error("session `{0}` already has a running prompt")]
    BusySession(String),
    /// A resume request used a different working directory.
    #[error("session `{0}` belongs to a different working directory")]
    CwdMismatch(String),
    /// Pagination cursor was not produced by this store.
    #[error("invalid session list cursor `{0}`")]
    InvalidCursor(String),
}

/// One page of session-list results.
#[derive(Debug)]
pub struct Page<'a, Update> {
    /// Sessions in creation order.
    pub sessions: Vec<&'a Session<Update>>,
    /// Cursor for the next page, when one exists.
    pub next_cursor: Option<String>,
}

/// Stored state for one session.
#[derive(Debug)]
pub struct Session<Update> {
    /// Protocol-facing session id.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Primary working directory.
    pub cwd: PathBuf,
    /// Additional workspace roots.
    pub additional_directories: Vec<PathBuf>,
    /// Number of MCP servers the client attached to the session.
    pub mcp_servers: usize,
    /// Current v1 mode (v2 exposes options only).
    pub mode: String,
    /// Current config option values.
    pub config: SessionConfig,
    /// Whether the session accepts prompts.
    pub active: bool,
    /// Whether a prompt worker is currently running.
    pub busy: bool,
    /// Whether the running prompt was cancelled.
    pub cancelled: bool,
    /// Updates sent during the session, in order.
    pub history: Vec<Update>,
    /// ISO 8601 timestamp of the last activity.
    pub updated_at: String,
}

/// In-memory session registry.
#[derive(Debug)]
pub struct Store<Update> {
    page_size: usize,
    next_id: u64,
    sessions: Vec<Session<Update>>,
}

impl<Update> Store<Update> {
    /// Create an empty store that returns at most `page_size` sessions per
    /// `session/list` page.
    #[must_use]
    pub fn new(page_size: usize) -> Self {
        Self {
            page_size: page_size.max(1),
            next_id: 0,
            sessions: Vec::new(),
        }
    }

    /// Create a session and return its id.
    pub fn create(
        &mut self,
        cwd: PathBuf,
        additional_directories: Vec<PathBuf>,
        mcp_servers: usize,
    ) -> String {
        self.next_id += 1;
        let id = format!("test-session-{}", self.next_id);
        self.sessions.push(Session {
            id: id.clone(),
            title: format!("ACP test session {}", self.next_id),
            cwd,
            additional_directories,
            mcp_servers,
            mode: super::MODE_ASK.to_string(),
            config: SessionConfig::default(),
            active: true,
            busy: false,
            cancelled: false,
            history: Vec::new(),
            updated_at: now(),
        });
        id
    }

    /// Look up a session.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownSession`] when the id is not registered.
    pub fn get(&self, id: &str) -> Result<&Session<Update>, Error> {
        self.sessions
            .iter()
            .find(|session| session.id == id)
            .ok_or_else(|| Error::UnknownSession(id.to_string()))
    }

    /// Look up a session mutably.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownSession`] when the id is not registered.
    pub fn get_mut(&mut self, id: &str) -> Result<&mut Session<Update>, Error> {
        self.sessions
            .iter_mut()
            .find(|session| session.id == id)
            .ok_or_else(|| Error::UnknownSession(id.to_string()))
    }

    /// Mark a session busy before its prompt worker starts.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownSession`], [`Error::ClosedSession`], or
    /// [`Error::BusySession`] when the prompt cannot start.
    pub fn begin_prompt(&mut self, id: &str) -> Result<(), Error> {
        let session = self.get_mut(id)?;
        if !session.active {
            return Err(Error::ClosedSession(id.to_string()));
        }
        if session.busy {
            return Err(Error::BusySession(id.to_string()));
        }
        session.busy = true;
        session.cancelled = false;
        session.updated_at = now();
        Ok(())
    }

    /// Clear prompt bookkeeping after the worker stops.
    pub fn finish_prompt(&mut self, id: &str) {
        if let Ok(session) = self.get_mut(id) {
            session.busy = false;
            session.cancelled = false;
            session.updated_at = now();
        }
    }

    /// Request cancellation of the running prompt, if any.
    pub fn cancel(&mut self, id: &str) {
        if let Ok(session) = self.get_mut(id)
            && session.busy
        {
            session.cancelled = true;
        }
    }

    /// Whether the running prompt for `id` has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self, id: &str) -> bool {
        self.get(id).is_ok_and(|session| session.cancelled)
    }

    /// Whether the session is accepting prompts right now.
    #[must_use]
    pub fn is_active(&self, id: &str) -> bool {
        self.get(id).is_ok_and(|session| session.active)
    }

    /// Append a typed update to the session history.
    pub fn record(&mut self, id: &str, update: Update) {
        if let Ok(session) = self.get_mut(id) {
            session.history.push(update);
            session.updated_at = now();
        }
    }

    /// Return one page of sessions, optionally filtered by working directory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidCursor`] when `cursor` is not a decimal offset.
    pub fn page(
        &self,
        cwd: Option<&Path>,
        cursor: Option<&str>,
    ) -> Result<Page<'_, Update>, Error> {
        let start = match cursor {
            None => 0,
            Some(cursor) => cursor
                .parse::<usize>()
                .map_err(|_| Error::InvalidCursor(cursor.to_string()))?,
        };
        let matching = self
            .sessions
            .iter()
            .filter(|session| cwd.is_none_or(|cwd| session.cwd == cwd))
            .collect::<Vec<_>>();
        let end = start.saturating_add(self.page_size).min(matching.len());
        let sessions = matching
            .get(start.min(matching.len())..end)
            .unwrap_or_default()
            .to_vec();
        let next_cursor = (end < matching.len()).then(|| end.to_string());
        Ok(Page {
            sessions,
            next_cursor,
        })
    }

    /// Close a session, keeping it available for listing and resume.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownSession`] when the id is not registered.
    pub fn close(&mut self, id: &str) -> Result<(), Error> {
        let session = self.get_mut(id)?;
        session.active = false;
        if session.busy {
            session.cancelled = true;
        }
        session.updated_at = now();
        Ok(())
    }

    /// Delete a session from the registry.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownSession`] when the id is not registered.
    pub fn delete(&mut self, id: &str) -> Result<(), Error> {
        let index = self
            .sessions
            .iter()
            .position(|session| session.id == id)
            .ok_or_else(|| Error::UnknownSession(id.to_string()))?;
        if self.sessions[index].busy {
            return Err(Error::BusySession(id.to_string()));
        }
        self.sessions.remove(index);
        Ok(())
    }

    /// Reopen a stored session for another prompt turn.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownSession`] or [`Error::CwdMismatch`] when the
    /// session cannot be resumed as requested.
    pub fn resume(
        &mut self,
        id: &str,
        cwd: &Path,
        additional_directories: Vec<PathBuf>,
        mcp_servers: usize,
    ) -> Result<(), Error> {
        let session = self.get_mut(id)?;
        if session.busy {
            return Err(Error::BusySession(id.to_string()));
        }
        if session.cwd != cwd {
            return Err(Error::CwdMismatch(id.to_string()));
        }
        session.active = true;
        session.busy = false;
        session.cancelled = false;
        session.additional_directories = additional_directories;
        session.mcp_servers = mcp_servers;
        session.updated_at = now();
        Ok(())
    }

    /// Copy a session into a new one and return the new id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownSession`] when the source is not registered.
    pub fn fork(
        &mut self,
        source_id: &str,
        cwd: PathBuf,
        additional_directories: Vec<PathBuf>,
        mcp_servers: usize,
    ) -> Result<String, Error>
    where
        Update: Clone,
    {
        let source = self
            .sessions
            .iter()
            .find(|session| session.id == source_id)
            .cloned()
            .ok_or_else(|| Error::UnknownSession(source_id.to_string()))?;
        if source.busy {
            return Err(Error::BusySession(source_id.to_string()));
        }
        self.next_id += 1;
        let id = format!("test-session-{}", self.next_id);
        self.sessions.push(Session {
            id: id.clone(),
            title: format!("{} (fork)", source.title),
            cwd,
            additional_directories,
            mcp_servers,
            mode: source.mode,
            config: source.config,
            active: true,
            busy: false,
            cancelled: false,
            history: source.history,
            updated_at: now(),
        });
        Ok(id)
    }
}

impl<Update> Clone for Session<Update>
where
    Update: Clone,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            additional_directories: self.additional_directories.clone(),
            mcp_servers: self.mcp_servers,
            mode: self.mode.clone(),
            config: self.config.clone(),
            active: self.active,
            busy: self.busy,
            cancelled: self.cancelled,
            history: self.history.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

/// Format the current UTC time as RFC 3339.
#[must_use]
pub fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("RFC 3339 is a well-known, always-valid format")
}
