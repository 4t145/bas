//! Agent-driven ACP authentication with an owned, isolated connection.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use agent_client_protocol::{
    Agent, Client, ConnectionTo, UntypedMessage, on_receive_notification, on_receive_request,
    schema::v1,
};
use anyhow::Context;
use basr::{
    authentication::{Execution, Method},
    service::pb,
};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc, oneshot};

use super::CheckResult;
use crate::agent::AgentSpec;

#[cfg(all(test, unix))]
mod tests;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Selected authentication method is not advertised by this agent")]
    MethodNotAdvertised,
    #[error("Authentication method changed; check the agent and retry")]
    MethodChanged,
    #[error("Terminal authentication requires an interactive terminal")]
    TerminalMethod,
    #[error("ACP authentication connection closed before completing")]
    ConnectionClosed,
    #[error("ACP authentication result was not received")]
    ResultLost,
    #[error("ACP authentication timed out")]
    Timeout,
    #[error("Interactive authentication was cancelled")]
    Cancelled,
    #[error("Agent cancelled this elicitation request")]
    PeerCancelled,
    #[error("Unknown or completed elicitation request")]
    UnknownElicitation,
    #[error("Unexpected authentication input")]
    UnexpectedInput,
    #[error("Terminal authentication exited with status {0}")]
    TerminalExit(i32),
}

struct Pending {
    request: basr::authentication::elicitation::Request,
    response: oneshot::Sender<Value>,
}

/// Connection-local correlation state. Inputs and outputs are never persisted.
struct Interaction {
    events: mpsc::Sender<Result<pb::AuthenticationEvent, tonic::Status>>,
    pending: Mutex<HashMap<String, Pending>>,
    urls: Mutex<HashMap<String, bool>>,
    requests: Mutex<HashSet<String>>,
    sessions: Mutex<HashSet<String>>,
    terminal: Mutex<Option<oneshot::Sender<i32>>>,
}

impl Interaction {
    async fn event(&self, kind: pb::authentication_event::Kind) -> anyhow::Result<()> {
        self.events
            .send(Ok(pb::AuthenticationEvent { kind: Some(kind) }))
            .await
            .map_err(|_| Error::Cancelled)?;
        Ok(())
    }

    async fn request(
        &self,
        raw: Value,
        cancellation: agent_client_protocol::RequestCancellation,
    ) -> anyhow::Result<Value> {
        let request = basr::authentication::elicitation::parse_request(&raw)?;
        if let Some(id) = request.scope.get("requestId") {
            anyhow::ensure!(
                self.requests.lock().await.contains(&id.to_string()),
                "Elicitation scope does not identify a live ACP request"
            );
        }
        if let Some(id) = request.scope.get("sessionId").and_then(Value::as_str) {
            anyhow::ensure!(
                self.sessions.lock().await.contains(id),
                "Elicitation scope does not identify a session on this ACP connection"
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        let url_id = raw
            .get("elicitationId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(url_id) = &url_id {
            let mut urls = self.urls.lock().await;
            anyhow::ensure!(
                !urls.contains_key(url_id),
                "Duplicate connection-scoped elicitation ID"
            );
            urls.insert(url_id.clone(), false);
        }
        let (send, receive) = oneshot::channel();
        self.pending.lock().await.insert(
            id.clone(),
            Pending {
                request,
                response: send,
            },
        );
        self.event(pb::authentication_event::Kind::Elicitation(
            pb::PendingElicitation {
                request_id: id.clone(),
                request: Some((&raw).into()),
                answered: false,
            },
        ))
        .await?;
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                self.pending.lock().await.remove(&id);
                if let Some(url_id) = &url_id { self.urls.lock().await.remove(url_id); }
                self.event(pb::authentication_event::Kind::ElicitationCancelled(id)).await?;
                return Err(Error::PeerCancelled.into());
            }
            response = receive => response.map_err(|_| Error::Cancelled)?,
        };
        if let Some(url_id) = url_id {
            let mut urls = self.urls.lock().await;
            if result["action"] == "accept" {
                if let Some(accepted) = urls.get_mut(&url_id) {
                    *accepted = true;
                }
            } else {
                urls.remove(&url_id);
            }
        }
        Ok(result)
    }

    async fn complete(&self, raw: Value) -> anyhow::Result<()> {
        let Some(id) = raw.get("elicitationId").and_then(Value::as_str) else {
            anyhow::bail!("Elicitation completion requires an ID");
        };
        let mut urls = self.urls.lock().await;
        if urls.get(id) != Some(&true) {
            return Ok(());
        }
        urls.remove(id);
        drop(urls);
        self.event(pb::authentication_event::Kind::ElicitationComplete(
            (&raw).into(),
        ))
        .await
    }

    async fn input(&self, input: pb::AuthenticationInput) -> anyhow::Result<()> {
        match input.kind {
            Some(pb::authentication_input::Kind::ElicitationResponse(answer)) => {
                let response: Value = answer.response.ok_or(Error::UnexpectedInput)?.decode()?;
                let mut pending = self.pending.lock().await;
                let current = pending
                    .get(&answer.request_id)
                    .ok_or(Error::UnknownElicitation)?;
                basr::authentication::elicitation::validate_response(&current.request, &response)?;
                let current = pending
                    .remove(&answer.request_id)
                    .ok_or(Error::UnknownElicitation)?;
                current
                    .response
                    .send(response)
                    .map_err(|_| Error::Cancelled)?;
                Ok(())
            }
            Some(pb::authentication_input::Kind::TerminalResult(result)) => {
                self.terminal
                    .lock()
                    .await
                    .take()
                    .ok_or(Error::UnexpectedInput)?
                    .send(result.exit_code)
                    .map_err(|_| Error::Cancelled)?;
                Ok(())
            }
            _ => Err(Error::UnexpectedInput.into()),
        }
    }
}

enum Outcome {
    Check(CheckResult),
    Terminal(Method),
}

/// Authenticates using a freshly advertised method, then checks session readiness.
/// Dropping this future cancels and reaps its owned ACP process group.
///
/// # Errors
/// Reports unavailable executables, unusable methods, ACP failures and timeouts.
#[cfg(test)]
pub async fn authenticate(
    spec: &AgentSpec,
    method_id: &str,
    timeout: Duration,
) -> anyhow::Result<CheckResult> {
    authenticate_expected(spec, method_id, None, timeout).await
}

/// Authenticate through the unary API with optional raw-method freshness checking.
///
/// # Errors
/// Reports stale descriptors, unusable methods, ACP failures and timeouts.
pub async fn authenticate_expected(
    spec: &AgentSpec,
    method_id: &str,
    expected: Option<Value>,
    timeout: Duration,
) -> anyhow::Result<CheckResult> {
    match connection(spec, method_id, expected, None, timeout).await? {
        Outcome::Check(result) => Ok(result),
        Outcome::Terminal(_) => Err(Error::TerminalMethod.into()),
    }
}

/// Interactive authentication owns all ACP processes until its stream is dropped.
/// Raw descriptors are revalidated against fresh discovery before execution.
///
/// # Errors
/// Reports stale descriptors, invalid responses, cancellation and failed agents.
pub async fn interactive(
    spec: &AgentSpec,
    request: &pb::AuthenticateAgentRequest,
    events: mpsc::Sender<Result<pb::AuthenticationEvent, tonic::Status>>,
    mut inputs: mpsc::Receiver<pb::AuthenticationInput>,
    timeout: Duration,
) -> anyhow::Result<CheckResult> {
    let expected = if request.expected_method_json.is_empty() {
        None
    } else {
        Some(
            serde_json::from_slice::<Value>(&request.expected_method_json)
                .context("Expected authentication method must be JSON")?,
        )
    };
    let interaction = Arc::new(Interaction {
        events,
        pending: Mutex::default(),
        urls: Mutex::default(),
        requests: Mutex::default(),
        sessions: Mutex::default(),
        terminal: Mutex::default(),
    });
    let operation = async {
        let outcome = connection(
            spec,
            &request.method_id,
            expected,
            Some(Arc::clone(&interaction)),
            timeout,
        )
        .await?;
        interaction.pending.lock().await.clear();
        interaction.urls.lock().await.clear();
        interaction.requests.lock().await.clear();
        interaction.sessions.lock().await.clear();
        match outcome {
            Outcome::Check(result) => Ok(result),
            Outcome::Terminal(method) => {
                let program =
                    super::resolve_program(spec).context("Agent executable is missing")?;
                let invocation =
                    crate::management::authentication_invocation(spec, &program, &method)?;
                let (send, receive) = oneshot::channel();
                *interaction.terminal.lock().await = Some(send);
                interaction
                    .event(pb::authentication_event::Kind::Terminal(
                        (&serde_json::to_value(invocation)?).into(),
                    ))
                    .await?;
                let exit = receive.await.map_err(|_| Error::Cancelled)?;
                if exit != 0 {
                    return Err(Error::TerminalExit(exit).into());
                }
                super::check_connection(spec, timeout)
                    .await
                    .map_err(anyhow::Error::msg)
            }
        }
    };
    tokio::pin!(operation);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            result = &mut operation => return result,
            () = interaction.events.closed() => return Err(Error::Cancelled.into()),
            input = inputs.recv() => interaction.input(input.ok_or(Error::Cancelled)?).await?,
            () = &mut deadline => return Err(Error::Timeout.into()),
        }
    }
}

async fn connection(
    spec: &AgentSpec,
    method_id: &str,
    expected: Option<Value>,
    interaction: Option<Arc<Interaction>>,
    timeout: Duration,
) -> anyhow::Result<Outcome> {
    let errors = super::probe::ErrorCapture::default();
    let capture = errors.clone();
    let transport = super::convert::agent(spec)
        .context("Executable could not be resolved")?
        .with_debug(move |line, direction| capture.observe(line, direction));
    let directory =
        tempfile::tempdir().context("Could not create isolated authentication directory")?;
    let cwd = directory.path().to_owned();
    let method_id = method_id.to_owned();
    let (ready, mut received) = oneshot::channel();
    let elicitation = interaction.clone();
    let completion = interaction.clone();
    let connection = Client
        .builder()
        .name("basrd-authenticate")
        .on_receive_request(
            async move |_request: v1::CreateTerminalRequest, _responder, _cx| {
                Err::<(), _>(denied("Authentication checks deny terminal access"))
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
                Err::<(), _>(denied("Authentication checks deny file access"))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::WriteTextFileRequest, _responder, _cx| {
                Err::<(), _>(denied("Authentication checks deny file access"))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |request: UntypedMessage, responder, cx| {
                if request.method != "elicitation/create" {
                    return Err(agent_client_protocol::Error::method_not_found());
                }
                let Some(interaction) = elicitation.clone() else {
                    responder.respond(serde_json::json!({"action":"cancel"}))?;
                    return Ok(());
                };
                let cancellation = responder.cancellation();
                cx.spawn(async move {
                    match interaction.request(request.params, cancellation).await {
                        Ok(response) => responder.respond(response)?,
                        Err(error) => {
                            let error = if error
                                .downcast_ref::<Error>()
                                .is_some_and(|error| matches!(error, Error::PeerCancelled))
                            {
                                agent_client_protocol::Error::request_cancelled()
                            } else {
                                agent_client_protocol::Error::invalid_params()
                                    .data(error.to_string())
                            };
                            responder.respond_with_error(error)?;
                        }
                    }
                    Ok(())
                })?;
                Ok(())
            },
            on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: UntypedMessage, _cx| {
                if notification.method == "elicitation/complete"
                    && let Some(interaction) = &completion
                {
                    interaction
                        .complete(notification.params)
                        .await
                        .map_err(|error| {
                            agent_client_protocol::Error::internal_error().data(error.to_string())
                        })?;
                }
                Ok(())
            },
            on_receive_notification!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            let result =
                authenticate_connection(cx, method_id, expected, interaction, cwd, errors).await;
            let _ = ready.send(result);
            std::future::pending::<()>().await;
            Ok(())
        });
    tokio::pin!(connection);
    tokio::select! {
        biased;
        result = &mut received => result.map_err(|_| Error::ResultLost)?,
        _ = &mut connection => Err(Error::ConnectionClosed.into()),
        () = tokio::time::sleep(timeout) => Err(Error::Timeout.into()),
    }
}

fn denied(message: &'static str) -> agent_client_protocol::Error {
    agent_client_protocol::Error::into_internal_error(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message,
    ))
}

async fn request(
    cx: &ConnectionTo<Agent>,
    method: &str,
    params: Value,
    interaction: Option<&Arc<Interaction>>,
    errors: &super::probe::ErrorCapture,
) -> Result<Value, super::probe::AuthenticationError> {
    let message = UntypedMessage::new(method, params).map_err(|error| errors.take("", error))?;
    let sent = cx.send_request(message);
    let id = serde_json::to_value(sent.id())
        .map_err(|error| errors.take("", error.into()))?
        .to_string();
    if let Some(interaction) = interaction {
        interaction.requests.lock().await.insert(id.clone());
    }
    let result = sent.block_task().await;
    if let Some(interaction) = interaction {
        interaction.requests.lock().await.remove(&id);
        if method == "session/new"
            && let Ok(response) = &result
            && let Some(id) = response.get("sessionId").and_then(Value::as_str)
        {
            interaction.sessions.lock().await.insert(id.to_owned());
        }
    }
    result.map_err(|error| errors.take(&id, error))
}

async fn authenticate_connection(
    cx: ConnectionTo<Agent>,
    method_id: String,
    expected: Option<Value>,
    interaction: Option<Arc<Interaction>>,
    cwd: PathBuf,
    errors: super::probe::ErrorCapture,
) -> anyhow::Result<Outcome> {
    use agent_client_protocol::JsonRpcMessage;
    let init_request =
        v1::InitializeRequest::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .client_capabilities(super::probe::capabilities())
            .to_untyped_message()?;
    let initialized = request(
        &cx,
        "initialize",
        init_request.params,
        interaction.as_ref(),
        &errors,
    )
    .await
    .context("ACP initialization failed")?;
    let _: v1::InitializeResponse = serde_json::from_value(initialized.clone())
        .context("Invalid ACP initialization response")?;
    let session_request = v1::NewSessionRequest::new(cwd).to_untyped_message()?;
    let discovery = request(
        &cx,
        "session/new",
        session_request.params.clone(),
        interaction.as_ref(),
        &errors,
    )
    .await;
    let error = match discovery {
        Ok(_) => None,
        Err(error) if error.code == agent_client_protocol::ErrorCode::AuthRequired => Some(error),
        Err(error) => return Err(error).context("Session discovery failed before authentication"),
    };
    let raw_error = error.as_ref().map(serde_json::to_value).transpose()?;
    let methods = basr::authentication::methods(&initialized, raw_error.as_ref())?;
    let method = methods
        .into_iter()
        .find(|method| method.id == method_id)
        .ok_or(Error::MethodNotAdvertised)?;
    if let Some(expected) = expected {
        let unchanged = expected == method.raw
            || serde_json::from_value::<Method>(expected).is_ok_and(|expected| expected == method);
        if !unchanged {
            return Err(Error::MethodChanged.into());
        }
    }
    match &method.execution {
        Execution::Terminal(_) => return Ok(Outcome::Terminal(method)),
        Execution::Unsupported { reason } => {
            anyhow::bail!("Authentication method is unsupported: {reason}")
        }
        Execution::Agent => {}
    }
    request(
        &cx,
        "authenticate",
        v1::AuthenticateRequest::new(method_id)
            .to_untyped_message()?
            .params,
        interaction.as_ref(),
        &errors,
    )
    .await
    .context("Agent authentication failed")?;
    match request(
        &cx,
        "session/new",
        session_request.params,
        interaction.as_ref(),
        &errors,
    )
    .await
    {
        Ok(_) => Ok(Outcome::Check(CheckResult::Connected(initialized))),
        Err(error) if error.code == agent_client_protocol::ErrorCode::AuthRequired => Ok(
            Outcome::Check(CheckResult::AuthenticationRequired(initialized, error)),
        ),
        Err(error) => Err(error).context("Session creation failed after authentication"),
    }
}
