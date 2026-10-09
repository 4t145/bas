//! Read-only host-side connection checks. No registry session or prompt is created.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::{
    Agent, Client, ConnectionTo, JsonRpcMessage, UntypedMessage, on_receive_request,
    schema::{ProtocolVersion, v1},
};

use crate::agent::AgentSpec;

/// Resolve exactly as the daemon will launch, never using a shell or GUI PATH.
/// Relative paths (including relative PATH entries) use the daemon's cwd.
pub fn resolve_program(spec: &AgentSpec) -> Option<PathBuf> {
    let command = spec.command()?;
    let path = spec
        .env
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"));
    which::which_in(&command.program, path, std::env::current_dir().ok()?).ok()
}

/// A protocol connection can succeed before the agent is ready to create sessions.
pub enum CheckResult {
    Connected(serde_json::Value),
    AuthenticationRequired(serde_json::Value, AuthenticationError),
}

/// Keeps the complete JSON-RPC error object alongside its SDK classification.
#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
pub struct AuthenticationError {
    pub raw: serde_json::Value,
    #[serde(skip)]
    error: Box<v1::Error>,
}

impl AuthenticationError {
    pub(super) fn into_parts(self) -> (v1::Error, serde_json::Value) {
        (*self.error, self.raw)
    }
}

impl std::ops::Deref for AuthenticationError {
    type Target = v1::Error;
    fn deref(&self) -> &Self::Target {
        &self.error
    }
}
impl std::fmt::Display for AuthenticationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for AuthenticationError {}
impl From<&AuthenticationError> for basr::service::pb::AcpPayload {
    fn from(error: &AuthenticationError) -> Self {
        (&error.raw).into()
    }
}

/// Public transport observation captures extension fields before SDK error decoding.
#[derive(Clone, Default)]
pub(super) struct ErrorCapture(Arc<Mutex<BTreeMap<String, serde_json::Value>>>);
impl ErrorCapture {
    pub(super) fn observe(&self, line: &str, direction: agent_client_protocol::LineDirection) {
        const MAX_CAPTURED_ERRORS: usize = 32;
        if !matches!(direction, agent_client_protocol::LineDirection::Stdout)
            || line.len() > basr::service::MAX_MESSAGE_SIZE
        {
            return;
        }
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        let (Some(id), Some(error)) = (message.get("id"), message.get("error")) else {
            return;
        };
        if !error.is_object() {
            return;
        }
        let mut errors = self.0.lock().expect("ACP error capture lock");
        if errors.len() >= MAX_CAPTURED_ERRORS {
            errors.pop_first();
        }
        errors.insert(id.to_string(), error.clone());
    }

    pub(super) fn take(&self, id: &str, error: v1::Error) -> AuthenticationError {
        let raw = self
            .0
            .lock()
            .expect("ACP error capture lock")
            .remove(id)
            .unwrap_or_else(|| serde_json::to_value(&error).expect("ACP error JSON"));
        AuthenticationError {
            raw,
            error: Box::new(error),
        }
    }
}

/// Capabilities remain identical during discovery and interactive authentication.
pub(super) fn capabilities() -> v1::ClientCapabilities {
    v1::ClientCapabilities::new()
        .auth(v1::AuthCapabilities::new().terminal(true))
        .elicitation(
            v1::ElicitationCapabilities::new()
                .form(v1::ElicitationFormCapabilities::new())
                .url(v1::ElicitationUrlCapabilities::new()),
        )
}

/// Captures the original initialize object before any SDK schema projection.
pub(super) async fn initialize(
    cx: &ConnectionTo<Agent>,
) -> Result<serde_json::Value, agent_client_protocol::Error> {
    let request = v1::InitializeRequest::new(ProtocolVersion::V1)
        .client_capabilities(capabilities())
        .to_untyped_message()?;
    let raw = cx
        .send_request(UntypedMessage::new("initialize", request.params)?)
        .block_task()
        .await?;
    let _: v1::InitializeResponse = serde_json::from_value(raw.clone())?;
    Ok(raw)
}

/// Runs initialize and newSession with no filesystem or host terminal access.
/// Dropping the SDK transport kills/reaps its owned process group, including on
/// timeout, request cancellation and daemon shutdown. Errors are deliberately
/// classified here rather than returning agent-controlled stderr or environment.
///
/// # Errors
/// Reports unavailable executables, temporary directory failure, rejected ACP
/// initialization/session creation or expiry of the daemon's startup deadline.
pub async fn check_connection(
    spec: &AgentSpec,
    timeout: Duration,
) -> Result<CheckResult, &'static str> {
    let errors = ErrorCapture::default();
    let capture = errors.clone();
    let transport = super::convert::agent(spec)
        .map_err(|_| "Executable could not be resolved")?
        .with_debug(move |line, direction| capture.observe(line, direction));
    let directory = tempfile::tempdir().map_err(|_| "Could not create isolated probe directory")?;
    let cwd = directory.path().to_owned();
    let (ready, mut received) = tokio::sync::oneshot::channel();
    let connection = Client
        .builder()
        .name("basrd-check")
        .on_receive_request(
            async move |_request: v1::CreateElicitationRequest, responder, _cx| {
                responder.respond(v1::CreateElicitationResponse::new(
                    v1::ElicitationAction::Cancel,
                ))?;
                Ok(())
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::CreateTerminalRequest, _responder, _cx| {
                Err::<(), _>(agent_client_protocol::Error::into_internal_error(
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Connection checks deny terminal access",
                    ),
                ))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::RequestPermissionRequest, responder, _cx| {
                responder.respond(v1::RequestPermissionResponse::new(
                    v1::RequestPermissionOutcome::Cancelled,
                ))?;
                Ok(())
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::ReadTextFileRequest, _responder, _cx| {
                Err::<(), _>(agent_client_protocol::Error::into_internal_error(
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Connection checks deny file access",
                    ),
                ))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::WriteTextFileRequest, _responder, _cx| {
                Err::<(), _>(agent_client_protocol::Error::into_internal_error(
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Connection checks deny file access",
                    ),
                ))
            },
            on_receive_request!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            let initialized = initialize(&cx).await?;
            let sent = cx.send_request(v1::NewSessionRequest::new(cwd));
            let id = serde_json::to_value(sent.id())?.to_string();
            let result = match sent.block_task().await {
                Ok(_) => CheckResult::Connected(initialized),
                Err(error) if error.code == agent_client_protocol::ErrorCode::AuthRequired => {
                    CheckResult::AuthenticationRequired(initialized, errors.take(&id, error))
                }
                Err(_) => return Ok(()),
            };
            let _ = ready.send(result);
            std::future::pending::<()>().await;
            Ok(())
        });
    // Keep transport in this future, not a detached task. Once the handshake is
    // observed, cancel transport immediately instead of waiting on a peer to exit.
    tokio::pin!(connection);
    tokio::select! {
        biased;
        result = &mut received => result.map_err(|_| "ACP initialization or newSession failed; check the agent's login"),
        _ = &mut connection => Err("ACP startup or handshake failed; verify the command and agent login"),
        () = tokio::time::sleep(timeout) => Err("ACP connection check timed out"),
    }
}
