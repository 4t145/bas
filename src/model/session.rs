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
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, strum::Display, strum::EnumString,
)]
#[strum(serialize_all = "snake_case")]
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
    /// Agent capabilities, authentication methods and implementation information.
    pub initialization: Option<crate::acp::InitializeResponse>,
    /// Current ACP options and modes, with the public daemon session id.
    pub acp: Option<crate::acp::NewSessionResponse>,
    /// Latest command catalog, independent of the bounded event history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_commands: Option<crate::acp::AvailableCommandsUpdate>,
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
            initialization: None,
            acp: None,
            available_commands: None,
        }
    }

    /// Records a lifecycle transition.
    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
        self.updated_at = Timestamp::now();
    }
}

impl SessionMeta {
    /// Applies ACP state updates so newly attached clients see current configuration.
    pub fn apply_acp(&mut self, update: &crate::acp::SessionUpdate) {
        use crate::acp::SessionUpdate;
        match update {
            SessionUpdate::AvailableCommandsUpdate(update) => {
                self.available_commands = Some(update.clone());
            }
            SessionUpdate::SessionInfoUpdate(info) => {
                if !info.title.is_undefined() {
                    self.title = info.title.clone().take();
                }
            }
            SessionUpdate::ConfigOptionUpdate(update) => {
                self.acp
                    .get_or_insert_with(|| crate::acp::NewSessionResponse::new(self.id.to_string()))
                    .config_options = Some(update.config_options.clone());
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                if let Some(modes) = self.acp.as_mut().and_then(|info| info.modes.as_mut()) {
                    modes.current_mode_id = update.current_mode_id.clone();
                }
            }
            _ => {}
        }
    }
}
