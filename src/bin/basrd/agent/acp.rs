//! ACP process transport and conversion to the daemon's turn-scoped updates.

use std::{path::PathBuf, sync::Arc, time::Duration};

use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectionTo, JsonRpcMessage, on_receive_notification,
    on_receive_request,
    schema::{ProtocolVersion, v1},
};
use async_trait::async_trait;
use basr::model::{AgentId, SessionId, StopReason, TurnId};
use futures::future::BoxFuture;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    Agent as AgentTrait, Error, Host, Kind, LaunchRequest, TurnUpdate, Update,
    driver::{Driver as DriverTrait, spawn_worker},
};

mod authentication;
mod convert;
mod probe;
pub use authentication::{authenticate_expected, interactive as authenticate_interactive};
pub(crate) use probe::AuthenticationError;
pub use probe::{CheckResult, check_connection, resolve_program};

#[cfg(test)]
mod tests;

const CLIENT_NAME: &str = "basrd";

#[derive(Default)]
pub struct Driver;
impl Driver {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DriverTrait for Driver {
    fn kind(&self) -> Kind {
        Kind::Acp
    }

    async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn AgentTrait>, Error> {
        self.open(request, Startup::Create).await
    }

    async fn restore(
        &self,
        request: LaunchRequest,
        acp_session_id: String,
    ) -> Result<Box<dyn AgentTrait>, Error> {
        self.open(request, Startup::Restore(acp_session_id.into()))
            .await
    }
}

enum Startup {
    Create,
    Restore(v1::SessionId),
}

impl Driver {
    /// # Errors
    /// Returns the ACP initialization or session setup failure without creating
    /// a replacement session for a failed restore.
    async fn open(
        &self,
        request: LaunchRequest,
        startup: Startup,
    ) -> Result<Box<dyn AgentTrait>, Error> {
        let (corrupt, mut corruption) = mpsc::channel(1);
        let errors = probe::ErrorCapture::default();
        let capture = errors.clone();
        let requests = Arc::new(std::sync::Mutex::new(
            std::collections::BTreeSet::<String>::new(),
        ));
        let transport_requests = Arc::clone(&requests);
        let transport =
            convert::agent(request.spec.as_ref())?.with_debug(move |line, direction| {
                tracing::trace!(?direction, bytes = line.len(), "ACP transport line");
                capture.observe(line, direction);
                if let Ok(message) = serde_json::from_str::<serde_json::Value>(line)
                    && let Some(id) = message.get("id")
                {
                    let mut pending = transport_requests.lock().expect("ACP request IDs lock");
                    if matches!(direction, agent_client_protocol::LineDirection::Stdin)
                        && message.get("method").is_some()
                    {
                        pending.insert(id.to_string());
                    } else if matches!(direction, agent_client_protocol::LineDirection::Stdout)
                        && message.get("method").is_none()
                    {
                        pending.remove(&id.to_string());
                    }
                }
                // The SDK answers parse errors and keeps reading. A daemon-owned peer
                // emitting damaged protocol data must not leave a prompt hung forever.
                // Ignore whitespace; valid unknown messages remain the SDK's concern.
                if matches!(direction, agent_client_protocol::LineDirection::Stdout)
                    && !line.trim().is_empty()
                    && let Err(error) = serde_json::from_str::<serde::de::IgnoredAny>(line)
                {
                    let _ = corrupt.try_send(error.to_string());
                }
            });
        let state = Arc::new(Mutex::new(State::default()));
        let (ready, opened) = oneshot::channel();
        let connection = Connection {
            transport,
            session: request.session,
            cwd: request.cwd,
            state: Arc::clone(&state),
            updates: request.updates.clone(),
            host: request.host,
            agent: request.spec.id.clone(),
            ready,
            startup,
            requests,
            errors,
        };
        let agent = request.spec.id.clone();
        let task = spawn_worker(request.updates.clone(), async move {
            tokio::select! {
                result = connection.run() => result,
                Some(message) = corruption.recv() => Err(Error::backend(agent, format!("malformed ACP transport: {message}"))),
            }
        });
        let opened = opened.await.map_err(|_| {
            Error::backend(
                request.spec.id.clone(),
                "ACP initialization failed; see daemon logs",
            )
        })??;
        Ok(Box::new(Session {
            agent: request.spec.id.clone(),
            connection: opened.connection,
            acp_session: opened.created.session_id.clone(),
            initialized: opened.initialized,
            initialization_raw: opened.initialization_raw,
            created: opened.created,
            state,
            updates: request.updates,
            connection_task: Some(task),
            turn_task: None,
            cancel: CancellationToken::new(),
            cancel_request: Arc::new(std::sync::Mutex::new(None)),
            cancellation_grace: request.spec.cancellation_grace,
        }))
    }
}

struct Connection {
    session: SessionId,
    transport: AcpAgent,
    cwd: PathBuf,
    state: Arc<Mutex<State>>,
    updates: mpsc::Sender<Update>,
    host: Arc<dyn Host>,
    agent: AgentId,
    ready: oneshot::Sender<Result<Opened, Error>>,
    startup: Startup,
    requests: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
    errors: probe::ErrorCapture,
}

struct ElicitationConnection {
    host: Arc<dyn Host>,
    session: SessionId,
    connection: uuid::Uuid,
}

impl Drop for ElicitationConnection {
    fn drop(&mut self) {
        self.host.cancel_elicitations(self.session, self.connection);
    }
}

impl Connection {
    /// # Errors
    /// Reports initialization, transport, and connection closure failures.
    async fn run(self) -> Result<(), Error> {
        let Connection {
            session,
            transport,
            cwd,
            state,
            updates,
            host,
            agent,
            ready,
            startup,
            requests,
            errors,
        } = self;
        let elicitation_connection = uuid::Uuid::new_v4();
        host.open_elicitation_connection(session, elicitation_connection);
        let _elicitation_connection = ElicitationConnection {
            host: Arc::clone(&host),
            session,
            connection: elicitation_connection,
        };
        let notification_state = Arc::clone(&state);
        let permission_host = Arc::clone(&host);
        let read_host = Arc::clone(&host);
        let elicitation_host = Arc::clone(&host);
        let completion_host = Arc::clone(&host);
        let elicitation_state = Arc::clone(&state);
        let startup_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let failed = Arc::clone(&startup_failed);
        let startup_agent = agent.clone();
        Client.builder().name(CLIENT_NAME)
        .on_receive_notification(async move |notification: v1::SessionNotification, _cx| {
            notification_state.lock().await.on_notification(notification, &updates).await
                .map_err(agent_client_protocol::Error::into_internal_error)
        }, on_receive_notification!())
        .on_receive_request(async move |mut request: v1::RequestPermissionRequest, responder, cx| {
            request.session_id = session.to_string().into();
            let host = Arc::clone(&permission_host);
            let cancellation = responder.cancellation();
            // Keep inbound dispatch free to receive cancellation and other requests
            // while the user decides. Dropping the host future clears its live card.
            cx.spawn(async move {
                let response = cancellation.run_until_cancelled(async move {
                    host.request_permission(request).await
                        .map(v1::RequestPermissionResponse::new)
                        .map_err(agent_client_protocol::Error::into_internal_error)
                }).await;
                responder.respond_with_result(response)?;
                Ok(())
            })?;
            Ok(())
        }, on_receive_request!())
        .on_receive_request(async move |mut request: v1::ReadTextFileRequest, responder, _cx| {
            request.session_id = session.to_string().into();
            let content = read_host.read_text_file(request).await
                .map_err(agent_client_protocol::Error::into_internal_error)?;
            responder.respond(v1::ReadTextFileResponse::new(content))?;
            Ok(())
        }, on_receive_request!())
        .on_receive_request(async move |mut request: v1::WriteTextFileRequest, responder, _cx| {
            request.session_id = session.to_string().into();
            host.write_text_file(request).await
                .map_err(agent_client_protocol::Error::into_internal_error)?;
            responder.respond(v1::WriteTextFileResponse::new())?;
            Ok(())
        }, on_receive_request!())
        .on_receive_request(async move |request: agent_client_protocol::UntypedMessage, responder, cx| {
            if request.method != "elicitation/create" {
                responder.respond_with_error(agent_client_protocol::Error::method_not_found())?;
                return Ok(());
            }
            if let Err(error) = basr::authentication::elicitation::parse_request(&request.params) {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params().data(error.to_string()))?;
                return Ok(());
            }
            if let Some(id) = request.params.get("requestId")
                && !requests.lock().expect("ACP request IDs lock").contains(&id.to_string())
            {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            }
            if let Some(requested) = request.params.get("sessionId").and_then(serde_json::Value::as_str) {
                let current = elicitation_state.lock().await.session.clone();
                if current.as_ref().is_none_or(|current| current.0.as_ref() != requested) {
                    responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                    return Ok(());
                }
            }
            let host = Arc::clone(&elicitation_host);
            let cancellation = responder.cancellation();
            cx.spawn(async move {
                let response = cancellation.run_until_cancelled(async move {
                    host.elicit(session, elicitation_connection, request.params).await
                        .map_err(|error| agent_client_protocol::Error::invalid_params().data(error.to_string()))
                }).await;
                responder.respond_with_result(response)?;
                Ok(())
            })?;
            Ok(())
        }, on_receive_request!())
        .on_receive_notification(async move |notification: agent_client_protocol::UntypedMessage, _cx| {
            if notification.method == "elicitation/complete"
                && let Some(id) = notification.params.get("elicitationId").and_then(serde_json::Value::as_str)
            { completion_host.complete_elicitation(session, elicitation_connection, id); }
            Ok(())
        }, on_receive_notification!())
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            let initialize = v1::InitializeRequest::new(ProtocolVersion::V1).client_capabilities(Self::capabilities());
            let initialize = agent_client_protocol::UntypedMessage::new("initialize", initialize)?;
            let initialization_raw = match cx.send_request(initialize).block_task().await {
                Ok(value) => value,
                Err(error) => {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = ready.send(Err(error.into()));
                    return Ok(());
                }
            };
            let initialized: v1::InitializeResponse = match serde_json::from_value(initialization_raw.clone()) {
                Ok(value) => value,
                Err(error) => {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = ready.send(Err(Error::with_source(startup_agent.clone(), "invalid ACP initialization", error)));
                    return Ok(());
                }
            };
            let opened_session = match startup {
                Startup::Create => {
                    let sent = cx.send_request(v1::NewSessionRequest::new(cwd));
                    let id = serde_json::to_value(sent.id())?.to_string();
                    sent.block_task().await.map_err(|error| startup_error(&errors, &id, error))
                }
                Startup::Restore(id) => {
                    // ACP requires session/load to finish history replay before its
                    // response; session/resume does not replay history.
                    // https://agentclientprotocol.com/protocol/session-setup#loading-sessions
                    state.lock().await.session = Some(id.clone());
                    if initialized.agent_capabilities.load_session {
                        let sent = cx.send_request(v1::LoadSessionRequest::new(id.clone(), cwd));
                        let request_id = serde_json::to_value(sent.id())?.to_string();
                        sent.block_task().await.map(|loaded| {
                                v1::NewSessionResponse::new(id)
                                    .modes(loaded.modes)
                                    .config_options(loaded.config_options)
                                    .meta(loaded.meta)
                            }).map_err(|error| startup_error(&errors, &request_id, error))
                    } else if initialized.agent_capabilities.session_capabilities.resume.is_some() {
                        let sent = cx.send_request(v1::ResumeSessionRequest::new(id.clone(), cwd));
                        let request_id = serde_json::to_value(sent.id())?.to_string();
                        sent.block_task().await.map(|resumed| {
                                v1::NewSessionResponse::new(id)
                                    .modes(resumed.modes)
                                    .config_options(resumed.config_options)
                                    .meta(resumed.meta)
                            }).map_err(|error| startup_error(&errors, &request_id, error))
                    } else {
                        Err(Error::Unsupported { agent: startup_agent.clone(), operation: "session/load or session/resume" })
                    }
                }
            };
            let created = match opened_session {
                Ok(value) => value,
                Err(error) => {
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = ready.send(Err(error));
                    return Ok(());
                }
            };
            state.lock().await.session = Some(created.session_id.clone());
            tracing::debug!(agent = ?initialized.agent_info, acp_session = %created.session_id, "ACP session ready");
            if ready.send(Ok(Opened { connection: cx.clone(), initialized, initialization_raw, created })).is_err() { return Ok(()); }
            cx.incoming_closed().await;
            Ok(())
        }).await.map_err(|error| Error::with_source(agent.clone(), "ACP connection failed", error))?;
        if startup_failed.load(std::sync::atomic::Ordering::Relaxed) {
            Ok(())
        } else {
            Err(Error::backend(agent, "ACP connection closed"))
        }
    }

    fn capabilities() -> v1::ClientCapabilities {
        v1::ClientCapabilities::new()
            .auth(v1::AuthCapabilities::new().terminal(true))
            .elicitation(
                v1::ElicitationCapabilities::new()
                    .form(v1::ElicitationFormCapabilities::new())
                    .url(v1::ElicitationUrlCapabilities::new()),
            )
            .fs(v1::FileSystemCapabilities::new()
                .read_text_file(true)
                .write_text_file(true))
            .terminal(false)
            .plan(v1::PlanCapabilities::new())
            .meta(v1::Meta::from_iter([(
                "terminal_output".into(),
                true.into(),
            )]))
            .session(
                v1::ClientSessionCapabilities::new()
                    .config_options(
                        v1::SessionConfigOptionsCapabilities::new()
                            .boolean(v1::BooleanConfigOptionCapabilities::new()),
                    )
                    .compaction(v1::CompactionCapabilities::new())
                    .notices(v1::NoticeCapabilities::new()),
            )
    }
}

fn startup_error(errors: &probe::ErrorCapture, id: &str, error: v1::Error) -> Error {
    if error.code == agent_client_protocol::ErrorCode::AuthRequired {
        let (error, raw) = errors.take(id, error).into_parts();
        Error::AcpRaw {
            error: Box::new(error),
            raw,
        }
    } else {
        Error::Acp(error)
    }
}

struct Opened {
    connection: ConnectionTo<Agent>,
    initialized: v1::InitializeResponse,
    initialization_raw: serde_json::Value,
    created: v1::NewSessionResponse,
}

struct Session {
    agent: AgentId,
    connection: ConnectionTo<Agent>,
    acp_session: v1::SessionId,
    initialized: v1::InitializeResponse,
    initialization_raw: serde_json::Value,
    created: v1::NewSessionResponse,
    state: Arc<Mutex<State>>,
    updates: mpsc::Sender<Update>,
    connection_task: Option<AbortOnDropHandle<()>>,
    turn_task: Option<AbortOnDropHandle<()>>,
    cancel: CancellationToken,
    cancel_request: Arc<std::sync::Mutex<Option<v1::CancelNotification>>>,
    cancellation_grace: Duration,
}

#[async_trait]
impl AgentTrait for Session {
    fn initialization_raw(&self) -> Option<serde_json::Value> {
        Some(self.initialization_raw.clone())
    }
    fn initialization(&self) -> Option<v1::InitializeResponse> {
        Some(self.initialized.clone())
    }
    fn session_info(&self) -> Option<v1::NewSessionResponse> {
        Some(self.created.clone())
    }

    fn acp_session_id(&self) -> Option<String> {
        Some(self.acp_session.to_string())
    }

    fn set_config_option(
        &self,
        mut request: v1::SetSessionConfigOptionRequest,
    ) -> BoxFuture<'static, Result<v1::SetSessionConfigOptionResponse, Error>> {
        request.session_id = self.acp_session.clone();
        let connection = self.connection.clone();
        let updates = self.updates.clone();
        Box::pin(async move {
            let (done, response) = oneshot::channel();
            connection
                .send_request(request)
                .on_receiving_result(async move |result| {
                    if let Ok(response) = &result {
                        commit(
                            &updates,
                            v1::SessionUpdate::ConfigOptionUpdate(
                                v1::ConfigOptionUpdate::new(response.config_options.clone())
                                    .meta(response.meta.clone()),
                            ),
                        )
                        .await?;
                    }
                    let _ = done.send(result);
                    Ok(())
                })?;
            response
                .await
                .map_err(|_| Error::backend("acp", "configuration response lost"))?
                .map_err(Error::Acp)
        })
    }

    fn set_mode(
        &self,
        mut request: v1::SetSessionModeRequest,
    ) -> BoxFuture<'static, Result<v1::SetSessionModeResponse, Error>> {
        request.session_id = self.acp_session.clone();
        let connection = self.connection.clone();
        let updates = self.updates.clone();
        let mode = request.mode_id.clone();
        Box::pin(async move {
            let (done, response) = oneshot::channel();
            connection
                .send_request(request)
                .on_receiving_result(async move |result| {
                    if result.is_ok() {
                        commit(
                            &updates,
                            v1::SessionUpdate::CurrentModeUpdate(v1::CurrentModeUpdate::new(mode)),
                        )
                        .await?;
                    }
                    let _ = done.send(result);
                    Ok(())
                })?;
            response
                .await
                .map_err(|_| Error::backend("acp", "mode response lost"))?
                .map_err(Error::Acp)
        })
    }

    fn prompt(&mut self, turn: TurnId, mut request: v1::PromptRequest) -> Result<(), Error> {
        request.session_id = self.acp_session.clone();
        // Keep RPC rejection distinct from a malformed success payload.
        let request = request.to_untyped_message()?;
        let connection = self.connection.clone();
        let updates = self.updates.clone();
        let state = Arc::clone(&self.state);
        let agent = self.agent.clone();
        let cancellation_grace = self.cancellation_grace;
        self.cancel = CancellationToken::new();
        self.cancel_request = Arc::new(std::sync::Mutex::new(None));
        let cancel_request = Arc::clone(&self.cancel_request);
        let cancel = self.cancel.clone();
        self.turn_task = Some(spawn_worker(updates.clone(), async move {
            {
                let mut state = state.lock().await;
                state.active = Some(turn);
            }
            if cancel.is_cancelled() {
                state.lock().await.active = None;
                let _ = updates
                    .send(Update::Turn {
                        turn,
                        update: TurnUpdate::Finished(Ok(v1::PromptResponse::new(
                            StopReason::Cancelled,
                        ))),
                    })
                    .await;
                return Ok(());
            }
            let response = connection.send_request(request).block_task();
            tokio::pin!(response);
            let response = tokio::select! {
                biased;
                result = &mut response => Ok(result),
                () = cancel.cancelled() => {
                    async {
                        let request = cancel_request.lock().expect("cancel request lock").take().expect("cancellation carries a request");
                        connection.send_notification(request)
                            .map_err(|error| Error::with_source(agent.clone(), "cannot send session/cancel", error))?;
                        tokio::time::timeout(cancellation_grace, response).await
                            .map_err(|_| Error::backend(agent.clone(), "ACP agent did not finish after cancellation"))
                    }.await
                }
            };
            // Serializes turn completion with notification callbacks, including bounded sends.
            let mut state = state.lock().await;
            state.active = None;
            let response = response?;
            if let Err(error) = &response
                && agent_client_protocol::is_incoming_transport_closed(error)
            {
                return Err(Error::with_source(
                    agent,
                    "ACP prompt transport closed",
                    error.clone(),
                ));
            }
            let response = match response {
                Ok(value) => Ok(serde_json::from_value::<v1::PromptResponse>(value).map_err(
                    |error| Error::with_source(agent, "malformed ACP prompt response", error),
                )?),
                Err(error) => Err(error.to_string()),
            };
            let _ = updates
                .send(Update::Turn {
                    turn,
                    update: TurnUpdate::Finished(response),
                })
                .await;
            Ok(())
        }));
        Ok(())
    }

    fn cancel(&mut self, mut request: v1::CancelNotification) -> Result<(), Error> {
        request.session_id = self.acp_session.clone();
        *self.cancel_request.lock().expect("cancel request lock") = Some(request);
        self.cancel.cancel();
        Ok(())
    }

    async fn shutdown(&mut self) {
        let tasks: Vec<_> = [self.turn_task.take(), self.connection_task.take()]
            .into_iter()
            .flatten()
            .collect();
        for task in &tasks {
            task.abort();
        }
        futures::future::join_all(tasks).await;
    }
}

/// The SDK's ordered callback barrier includes the session commit, not the caller's polling.
/// See <https://docs.rs/agent-client-protocol/3.3.0/agent_client_protocol/struct.SentRequest.html#method.on_receiving_result>.
///
/// # Errors
/// Returns an error if the session stops before accepting this state mutation.
async fn commit(
    updates: &mpsc::Sender<Update>,
    update: v1::SessionUpdate,
) -> Result<(), agent_client_protocol::Error> {
    let (committed, receipt) = oneshot::channel();
    updates
        .send(Update::Commit { update, committed })
        .await
        .map_err(agent_client_protocol::Error::into_internal_error)?;
    receipt
        .await
        .map_err(agent_client_protocol::Error::into_internal_error)
}

#[derive(Default)]
struct State {
    session: Option<v1::SessionId>,
    active: Option<TurnId>,
}

impl State {
    /// # Errors
    /// Rejects notifications routed to another ACP session.
    async fn on_notification(
        &mut self,
        notification: v1::SessionNotification,
        updates: &mpsc::Sender<Update>,
    ) -> Result<(), Error> {
        if self
            .session
            .as_ref()
            .is_some_and(|id| id != &notification.session_id)
        {
            return Err(Error::backend(
                "acp",
                "update references a different session",
            ));
        }
        let update = match self.active {
            Some(turn) => Update::Turn {
                turn,
                update: TurnUpdate::Acp(notification.update),
            },
            None => Update::Acp(notification.update),
        };
        let _ = updates.send(update).await;
        Ok(())
    }
}
