use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{AgentId, Cursor, Event, EventKind, SessionId, Timestamp, TurnId};

mod runtime;
pub use runtime::*;

#[cfg(test)]
mod tests;

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

/// Lifecycle values used by committed metadata and runtime views.
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

/// Bas registered the client-owned submission. This is not proof of ACP dispatch
/// or successful execution. The cursor identifies its committed start event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TurnReceipt {
    pub turn: TurnId,
    pub cursor: Cursor,
}

/// The last committed session metadata. Runtime health belongs to [`SessionSnapshot`].
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
    /// Additional filesystem roots; cwd remains the base for relative paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "super::Subagents::is_empty")]
    pub subagents: super::Subagents,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Agent-reported timestamp, separate from the daemon's event clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_updated_at: Option<Timestamp>,
    pub status: SessionStatus,
    /// Hidden from the ordinary list without stopping the backend or discarding history.
    #[serde(default)]
    pub archived: bool,
    /// Cursor of the most recent agent message chunk; metadata-only edits do not change it.
    #[serde(default)]
    pub last_reply: Cursor,
    /// Last event included in this metadata snapshot, independent of replay retention.
    #[serde(default)]
    pub revision: Cursor,
    /// Active turn at `revision`; a failed runtime may no longer be executing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<TurnId>,
    /// Agent capabilities, authentication methods and implementation information.
    pub initialization: Option<crate::acp::InitializeResponse>,
    /// Original ACP initialization, including unknown authentication descriptors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialization_raw: Option<serde_json::Value>,
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
            additional_directories: Vec::new(),
            subagents: super::Subagents::default(),
            created_at: now,
            updated_at: now,
            agent_updated_at: None,
            status: SessionStatus::Starting,
            archived: false,
            last_reply: Cursor::START,
            revision: Cursor::START,
            active_turn: None,
            initialization: None,
            initialization_raw: None,
            acp: None,
            available_commands: None,
        }
    }

    /// Records a lifecycle transition.
    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
        if status != SessionStatus::Working {
            self.active_turn = None;
        }
        self.updated_at = Timestamp::now();
    }

    /// Applies only events newer than this snapshot. Content replay has its own cursor.
    pub fn apply_event(&mut self, event: &Event) -> bool {
        if event.is_transient() || event.session != self.id || event.cursor <= self.revision {
            return false;
        }
        match &event.kind {
            EventKind::Acp(update) => {
                if matches!(update, crate::acp::SessionUpdate::AgentMessageChunk(_)) {
                    self.last_reply = event.cursor;
                }
                self.apply_acp(update);
            }
            EventKind::Subagent(notification) => {
                if let crate::acp::SessionUpdate::SubagentUpdate(update) = &notification.update {
                    let _ = self
                        .subagents
                        .apply(Some(notification.session_id.0.as_ref()), update);
                }
            }
            EventKind::TitleChanged { title } => self.title = Some(title.clone()),
            EventKind::ArchiveChanged { archived } => self.archived = *archived,
            EventKind::WorkspaceChanged {
                directories,
                config,
                subagents,
            } => {
                self.additional_directories = directories.clone();
                self.acp = Some(config.clone());
                self.subagents = subagents.clone();
            }
            EventKind::StatusChanged { status } => {
                self.status = *status;
                if *status != SessionStatus::Working {
                    self.active_turn = None;
                }
            }
            EventKind::TurnStarted { turn } => {
                self.status = SessionStatus::Working;
                self.active_turn = Some(*turn);
            }
            EventKind::TurnFinished { .. } | EventKind::TurnInterrupted { .. } => {
                self.status = SessionStatus::Idle;
                self.active_turn = None;
            }
            EventKind::Failed { .. } => {
                self.status = SessionStatus::Failed;
                self.active_turn = None;
            }
        }
        self.revision = event.cursor;
        if matches!(self.status, SessionStatus::Stopped | SessionStatus::Failed) {
            self.subagents.invalidate_activity();
        }
        self.updated_at = event.at;
        true
    }
}

impl SessionMeta {
    /// Applies ACP state updates so newly attached clients see current configuration.
    pub fn apply_acp(&mut self, update: &crate::acp::SessionUpdate) {
        use crate::acp::SessionUpdate;
        match update {
            SessionUpdate::SubagentUpdate(update) => {
                let _ = self.subagents.apply(None, update);
            }
            SessionUpdate::AvailableCommandsUpdate(update) => {
                self.available_commands = Some(update.clone());
            }
            SessionUpdate::SessionInfoUpdate(info) => {
                if !info.title.is_undefined() {
                    self.title = info.title.clone().take();
                }
                if !info.updated_at.is_undefined() {
                    match info.updated_at.value() {
                        Some(value) => {
                            if let Ok(parsed) = time::OffsetDateTime::parse(
                                value,
                                &time::format_description::well_known::Rfc3339,
                            ) && let Ok(millis) =
                                u64::try_from(parsed.unix_timestamp_nanos() / 1_000_000)
                            {
                                self.agent_updated_at = Some(Timestamp::from_millis(millis));
                            }
                        }
                        None => self.agent_updated_at = None,
                    }
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
