use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::model::{
    AgentId, ContentBlock, Delta, EventKind, Message, Owner, SessionId, StopReason, ToolCall,
    TurnId,
};

use super::{AgentSpec, Kind};

/// Failure raised by an agent backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The backend could not be started or died.
    #[error("agent backend {agent} failed: {message}")]
    Backend {
        agent: AgentId,
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    /// The backend is alive but cannot do what was asked.
    #[error("agent backend {agent} does not support {operation}")]
    Unsupported {
        agent: AgentId,
        operation: &'static str,
    },
}

impl Error {
    /// Builds a [`Error::Backend`] from a message.
    pub fn backend(agent: impl Into<AgentId>, message: impl Into<String>) -> Self {
        Self::Backend {
            agent: agent.into(),
            message: message.into(),
            source: None,
        }
    }

    /// Builds a [`Error::Backend`] that keeps the underlying cause.
    pub fn with_source(
        agent: impl Into<AgentId>,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Backend {
            agent: agent.into(),
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// What a backend reports while a turn is running.
///
/// Backends emit these instead of session-scoped [`crate::model::Event`]s: the
/// session layer owns cursors, timestamps, and status transitions.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// A chunk of a message that is still streaming.
    Delta(Delta),
    /// A finished message.
    Message(Message),
    /// A tool call changed state.
    ToolCall(ToolCall),
    /// The backend renamed the conversation.
    Title(String),
    /// A turn started.
    TurnStarted { turn: TurnId },
    /// A turn finished.
    TurnFinished { turn: TurnId, stop: StopReason },
}

impl From<Update> for EventKind {
    fn from(update: Update) -> Self {
        match update {
            Update::Delta(delta) => Self::Delta(delta),
            Update::Message(message) => Self::Message(message),
            Update::ToolCall(call) => Self::ToolCall(call),
            Update::Title(title) => Self::TitleChanged { title },
            Update::TurnStarted { turn } => Self::TurnStarted { turn },
            Update::TurnFinished { turn, stop } => Self::TurnFinished { turn, stop },
        }
    }
}

/// A live conversation with one agent backend.
#[async_trait]
pub trait Agent: Send + Sync {
    /// Submits a user turn.
    ///
    /// Returns as soon as the backend accepted the turn; progress and
    /// completion arrive as [`Update`]s on the channel given to
    /// [`Driver::launch`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the backend rejects the turn or has
    /// exited.
    async fn prompt(&self, content: Vec<ContentBlock>) -> Result<TurnId, Error>;

    /// Asks the backend to abandon the running turn.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when the backend cannot cancel.
    async fn cancel(&self) -> Result<(), Error>;

    /// Renames the conversation on the backend side, if it has a notion of
    /// titles.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when the backend cannot rename.
    async fn set_title(&self, title: &str) -> Result<(), Error>;

    /// Releases the backend and any child process.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the backend could not be shut down
    /// cleanly.
    async fn shutdown(&self) -> Result<(), Error>;
}

/// Everything a driver needs to start one conversation.
pub struct LaunchRequest {
    pub session: SessionId,
    pub spec: Arc<AgentSpec>,
    pub owner: Owner,
    /// Directory the backend should treat as the workspace.
    pub cwd: Option<PathBuf>,
    /// Initial title, when a client supplied one at creation time.
    pub title: Option<String>,
    /// Callbacks the backend uses to reach the daemon (files, permissions).
    pub host: Arc<dyn Host>,
    /// Channel the backend reports progress on.
    ///
    /// Unbounded on purpose: backends publish from protocol dispatch loops that
    /// must never block, and the session layer bounds what is retained.
    pub updates: mpsc::UnboundedSender<Update>,
}

/// Starts agent backends of one [`Kind`].
#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// The backend kind this driver handles.
    fn kind(&self) -> Kind;

    /// Starts one conversation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the process or in-process agent cannot
    /// be started.
    async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn Agent>, Error>;
}

/// Services a backend may call back into while it runs.
#[async_trait]
pub trait Host: Send + Sync {
    /// Asks the user to approve a tool call.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when no attached client can answer.
    async fn request_permission(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionOutcome, Error>;

    /// Reads a UTF-8 file on the backend's behalf.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the path cannot be read.
    async fn read_text_file(&self, request: FileReadRequest) -> Result<String, Error>;

    /// Writes a UTF-8 file on the backend's behalf.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the path cannot be written.
    async fn write_text_file(&self, request: FileWriteRequest) -> Result<(), Error>;
}

/// A tool call the backend wants the user to approve.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub session: SessionId,
    pub call: ToolCall,
    pub options: Vec<PermissionOption>,
}

/// One choice offered for a [`PermissionRequest`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionOption {
    pub id: String,
    pub label: String,
    pub kind: PermissionKind,
}

/// How long an approval lasts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// What the user decided.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionOutcome {
    Selected(PermissionOption),
    Cancelled,
}

/// A read the backend delegated to the daemon.
#[derive(Clone, Debug)]
pub struct FileReadRequest {
    pub session: SessionId,
    pub path: PathBuf,
    /// First line to return, 1-based, when the backend asks for a slice.
    pub line: Option<u32>,
    pub limit: Option<u32>,
}

/// A write the backend delegated to the daemon.
#[derive(Clone, Debug)]
pub struct FileWriteRequest {
    pub session: SessionId,
    pub path: PathBuf,
    pub content: String,
}

/// What [`LocalHost`] does with permission requests that no client answers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PermissionPolicy {
    /// Approve automatically. Suitable for a headless daemon whose clients
    /// trust the configured backends.
    #[default]
    Allow,
    /// Refuse automatically.
    Deny,
}

/// A [`Host`] that touches the local filesystem and applies a fixed
/// permission policy.
///
/// Frontends that want interactive approvals should provide their own [`Host`]
/// that routes [`PermissionRequest`]s to attached clients.
pub struct LocalHost {
    policy: PermissionPolicy,
}

impl LocalHost {
    #[must_use]
    pub fn new(policy: PermissionPolicy) -> Self {
        Self { policy }
    }
}

impl Default for LocalHost {
    fn default() -> Self {
        Self::new(PermissionPolicy::default())
    }
}

#[async_trait]
impl Host for LocalHost {
    async fn request_permission(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionOutcome, Error> {
        let want = match self.policy {
            PermissionPolicy::Allow => |kind: &PermissionKind| {
                matches!(
                    kind,
                    PermissionKind::AllowOnce | PermissionKind::AllowAlways
                )
            },
            PermissionPolicy::Deny => |kind: &PermissionKind| {
                matches!(
                    kind,
                    PermissionKind::RejectOnce | PermissionKind::RejectAlways
                )
            },
        };

        Ok(request
            .options
            .iter()
            .find(|option| want(&option.kind))
            .cloned()
            .map_or(PermissionOutcome::Cancelled, PermissionOutcome::Selected))
    }

    async fn read_text_file(&self, request: FileReadRequest) -> Result<String, Error> {
        let content = tokio::fs::read_to_string(&request.path)
            .await
            .map_err(|error| {
                Error::with_source(
                    "host",
                    format!("cannot read {}", request.path.display()),
                    error,
                )
            })?;

        Ok(match (request.line, request.limit) {
            (Some(line), limit) => slice_lines(&content, line, limit),
            (None, _) => content,
        })
    }

    async fn write_text_file(&self, request: FileWriteRequest) -> Result<(), Error> {
        if let Some(parent) = request.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                Error::with_source("host", format!("cannot create {}", parent.display()), error)
            })?;
        }

        tokio::fs::write(&request.path, request.content)
            .await
            .map_err(|error| {
                Error::with_source(
                    "host",
                    format!("cannot write {}", request.path.display()),
                    error,
                )
            })
    }
}

fn slice_lines(content: &str, line: u32, limit: Option<u32>) -> String {
    let start = usize::try_from(line.saturating_sub(1)).unwrap_or(0);
    let count = limit.map_or(usize::MAX, |limit| limit as usize);

    content
        .lines()
        .skip(start)
        .take(count)
        .collect::<Vec<_>>()
        .join("\n")
}
