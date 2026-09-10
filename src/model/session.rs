use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{AgentId, SessionId, Timestamp};

/// Used when neither `USER` nor `LOGNAME` is set, as in a bare systemd unit.
const UNKNOWN_USER: &str = "unknown";
/// Used when `HOSTNAME` is unset; `hostname` is not consulted to keep this
/// function free of process spawning.
const UNKNOWN_HOST: &str = "localhost";

/// Who asked for a session, used for auditing and for per-user routing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Owner {
    pub user: String,
    pub host: String,
}

impl Owner {
    /// Reads the current process owner from the environment.
    #[must_use]
    pub fn current() -> Self {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| UNKNOWN_USER.to_owned());
        let host = std::env::var("HOSTNAME").unwrap_or_else(|_| UNKNOWN_HOST.to_owned());
        Self { user, host }
    }
}

/// Lifecycle of a session, as observed by attached clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// The backend is still starting up.
    Starting,
    /// No turn is in flight.
    Idle,
    /// The agent is working on a turn.
    Working,
    /// The backend exited cleanly.
    Stopped,
    /// The backend or the daemon gave up on the session.
    Failed,
}

/// Everything the daemon records about a session besides its events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    /// Backend that owns the conversation.
    pub agent: AgentId,
    /// Weakly-typed label shown by frontends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// User and machine that created the session.
    pub owner: Owner,
    /// Working directory the agent was launched in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub status: SessionStatus,
}

impl SessionMeta {
    /// Creates metadata for a session that is about to be launched.
    #[must_use]
    pub fn new(id: SessionId, agent: AgentId, owner: Owner) -> Self {
        let now = Timestamp::now();
        Self {
            id,
            agent,
            title: None,
            owner,
            cwd: None,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Starting,
        }
    }

    /// Records a lifecycle transition.
    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
        self.updated_at = Timestamp::now();
    }
}
