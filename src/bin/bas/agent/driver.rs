use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use futures::FutureExt;
use tokio::sync::mpsc;
use tokio_util::task::AbortOnDropHandle;

pub use bas::acp::{
    PermissionOptionKind as PermissionKind, ReadTextFileRequest as FileReadRequest,
    RequestPermissionOutcome as PermissionOutcome, RequestPermissionRequest as PermissionRequest,
    WriteTextFileRequest as FileWriteRequest,
};
use bas::{
    acp,
    model::{AgentId, PromptResult, SessionId, TurnId},
};
use futures::future::BoxFuture;

use super::{AgentSpec, Kind};

/// Failure raised by an agent backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Acp(#[from] acp::Error),
    /// Original ACP error object retained before SDK projection.
    #[error("{error}")]
    AcpRaw {
        #[source]
        error: Box<acp::Error>,
        raw: serde_json::Value,
    },
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
    /// Formats the backend failure with its underlying causes for diagnostics.
    pub fn report(&self) -> String {
        let mut message = self.to_string();
        let mut source = std::error::Error::source(self);
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        message
    }

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

/// Updates are ordered by the backend and scoped to the turn that produced them.
#[derive(Debug)]
pub enum Update {
    Turn {
        turn: TurnId,
        update: TurnUpdate,
    },
    Acp(acp::SessionUpdate),
    /// Original child-scoped traffic from an announced subagent.
    Subagent(acp::SessionNotification),
    /// Ordered state mutation acknowledged after the session commits it.
    Commit {
        update: acp::SessionUpdate,
        committed: tokio::sync::oneshot::Sender<()>,
    },
    Workspace {
        directories: Vec<PathBuf>,
        config: acp::NewSessionResponse,
        subagents: bas::model::Subagents,
        committed: tokio::sync::oneshot::Sender<()>,
    },
    /// Marks that all earlier updates from this backend have been consumed.
    Barrier(tokio::sync::oneshot::Sender<()>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "Keep ACP chunks inline while passing them through the bounded update channel"
)]
pub enum TurnUpdate {
    Acp(acp::SessionUpdate),
    Finished(PromptResult),
    CancelledBeforeDispatch,
}

/// Backend calls only enqueue work; they must not block or re-enter the session.
/// Dropping a backend must cancel all tasks and release its child processes.
#[async_trait]
pub trait Agent: Send {
    /// Starts a turn whose id and exclusivity are owned by the session layer.
    ///
    /// # Errors
    /// Returns an error if the content is unsupported or the backend is unavailable.
    fn prompt(&mut self, turn: TurnId, request: acp::PromptRequest) -> Result<(), Error>;

    fn initialization(&self) -> Option<acp::InitializeResponse> {
        None
    }
    fn initialization_raw(&self) -> Option<serde_json::Value> {
        self.initialization()
            .map(|response| serde_json::to_value(response).expect("ACP initialization serializes"))
    }
    fn session_info(&self) -> Option<acp::NewSessionResponse> {
        None
    }

    /// Raw backend session ID to persist for a later [`Driver::restore`].
    /// This is distinct from the public [`SessionId`] assigned by the daemon.
    fn acp_session_id(&self) -> Option<String> {
        None
    }

    fn subagents(&self) -> bas::model::Subagents {
        bas::model::Subagents::default()
    }

    /// # Errors
    /// Returns Unsupported unless the backend can reload an existing session with extra roots.
    fn set_directories(&self, _directories: Vec<PathBuf>) -> BoxFuture<'static, Result<(), Error>> {
        Box::pin(async {
            Err(Error::Unsupported {
                agent: "backend".into(),
                operation: "additionalDirectories",
            })
        })
    }

    /// # Errors
    /// Returns Unsupported for backends without configurable options.
    fn set_config_option(
        &self,
        _request: acp::SetSessionConfigOptionRequest,
    ) -> BoxFuture<'static, Result<acp::SetSessionConfigOptionResponse, Error>> {
        Box::pin(async {
            Err(Error::Unsupported {
                agent: "backend".into(),
                operation: "session/set_config_option",
            })
        })
    }

    /// # Errors
    /// Returns Unsupported for backends without session modes.
    fn set_mode(
        &self,
        _request: acp::SetSessionModeRequest,
    ) -> BoxFuture<'static, Result<acp::SetSessionModeResponse, Error>> {
        Box::pin(async {
            Err(Error::Unsupported {
                agent: "backend".into(),
                operation: "session/set_mode",
            })
        })
    }

    /// Requests cancellation; completion still arrives through the update channel.
    ///
    /// # Errors
    /// Returns an error if cancellation cannot be sent to the backend.
    fn cancel(&mut self, request: acp::CancelNotification) -> Result<(), Error>;

    /// # Errors
    /// Returns Unsupported unless the backend advertises standard session closure.
    fn close_session(&self) -> BoxFuture<'static, Result<(), Error>> {
        Box::pin(async {
            Err(Error::Unsupported {
                agent: "backend".into(),
                operation: "session/close",
            })
        })
    }

    /// Stops and joins all backend tasks. Dropping this future must also stop them.
    async fn shutdown(&mut self);
}

pub struct LaunchRequest {
    pub session: SessionId,
    pub spec: Arc<AgentSpec>,
    pub cwd: PathBuf,
    pub additional_directories: Vec<PathBuf>,
    pub host: Arc<dyn Host>,
    pub updates: mpsc::Sender<Update>,
}

#[async_trait]
pub trait Driver: Send + Sync + 'static {
    fn kind(&self) -> Kind;

    /// Starts a backend. Cancellation must release any partially started process.
    ///
    /// # Errors
    /// Returns an error when the backend cannot be initialized.
    async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn Agent>, Error>;

    /// Loads an existing backend session by its raw ACP ID (not the public session ID).
    /// The default rejects restoration; it must never create a replacement session.
    ///
    /// # Errors
    /// Returns Unsupported if the backend has no restore implementation, or a
    /// backend/protocol error if initialization or loading fails.
    async fn restore(
        &self,
        request: LaunchRequest,
        _acp_session_id: String,
    ) -> Result<Box<dyn Agent>, Error> {
        Err(Error::Unsupported {
            agent: request.spec.id.clone(),
            operation: "session/load",
        })
    }
}

/// Report errors and panics while keeping cleanup within one owned task.
pub(super) fn spawn_worker(
    updates: mpsc::Sender<Update>,
    work: impl std::future::Future<Output = Result<(), Error>> + Send + 'static,
) -> AbortOnDropHandle<()> {
    AbortOnDropHandle::new(tokio::spawn(async move {
        let failure = match std::panic::AssertUnwindSafe(work).catch_unwind().await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error.report(),
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                format!("backend task panicked: {message}")
            }
        };
        tracing::warn!(error = %failure, "backend worker failed: {failure}");
        let _ = updates.send(Update::Failed(failure)).await;
    }))
}

/// Services a backend may call back into while it runs.
#[async_trait]
pub trait Host: Send + Sync {
    /// Registers the current ACP connection generation for a Bas session.
    fn open_elicitation_connection(&self, _session: SessionId, _connection: uuid::Uuid) {}

    /// Requests connection-scoped structured information from an attached frontend.
    ///
    /// # Errors
    /// Reports unavailable frontends or invalid elicitation descriptors.
    async fn elicit(
        &self,
        _session: SessionId,
        _connection: uuid::Uuid,
        _request: serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        Ok(serde_json::json!({"action":"cancel"}))
    }

    fn complete_elicitation(
        &self,
        _session: SessionId,
        _connection: uuid::Uuid,
        _elicitation_id: &str,
    ) {
    }
    fn cancel_elicitations(&self, _session: SessionId, _connection: uuid::Uuid) {}

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

/// What [`LocalHost`] does with permission requests that no client answers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum PermissionPolicy {
    /// Approve automatically. Suitable for a headless daemon whose clients
    /// trust the configured backends.
    Allow,
    /// Refuse automatically.
    #[default]
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
        tracing::debug!(session = %request.session_id, tool = %request.tool_call.tool_call_id, policy = ?self.policy, "applying permission policy");
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
            .map(|option| acp::SelectedPermissionOutcome::new(option.option_id.clone()))
            .map_or(PermissionOutcome::Cancelled, PermissionOutcome::Selected))
    }

    async fn read_text_file(&self, request: FileReadRequest) -> Result<String, Error> {
        tracing::debug!(session = %request.session_id, path = %request.path.display(), "backend file read");
        let content = tokio::fs::read_to_string(&request.path)
            .await
            .map_err(|error| {
                Error::with_source(
                    "host",
                    format!("cannot read {}", request.path.display()),
                    error,
                )
            })?;

        Ok(select_lines(&request, content))
    }

    async fn write_text_file(&self, request: FileWriteRequest) -> Result<(), Error> {
        tracing::debug!(session = %request.session_id, path = %request.path.display(), "backend file write");
        if let Some(parent) = request
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
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

fn select_lines(request: &FileReadRequest, content: String) -> String {
    if request.line.is_none() && request.limit.is_none() {
        return content;
    }
    let start = request.line.unwrap_or(1).saturating_sub(1) as usize;
    let count = request.limit.map_or(usize::MAX, |limit| limit as usize);
    content
        .lines()
        .skip(start)
        .take(count)
        .collect::<Vec<_>>()
        .join("\n")
}
