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

mod convert;

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
        let (corrupt, mut corruption) = mpsc::channel(1);
        let transport =
            AcpAgent::try_from(request.spec.as_ref())?.with_debug(move |line, direction| {
                tracing::trace!(?direction, bytes = line.len(), "ACP transport line");
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
        })?;
        Ok(Box::new(Session {
            agent: request.spec.id.clone(),
            connection: opened.connection,
            acp_session: opened.created.session_id.clone(),
            initialized: opened.initialized,
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
    ready: oneshot::Sender<Opened>,
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
        } = self;
        let notification_state = Arc::clone(&state);
        let permission_host = Arc::clone(&host);
        let read_host = Arc::clone(&host);
        Client.builder().name(CLIENT_NAME)
        .on_receive_notification(async move |notification: v1::SessionNotification, _cx| {
            notification_state.lock().await.on_notification(notification, &updates).await
                .map_err(agent_client_protocol::Error::into_internal_error)
        }, on_receive_notification!())
        .on_receive_request(async move |mut request: v1::RequestPermissionRequest, responder, _cx| {
            request.session_id = session.to_string().into();
            let response = permission_host.request_permission(request).await
                .map_err(agent_client_protocol::Error::into_internal_error)?;
            responder.respond(v1::RequestPermissionResponse::new(response))?;
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
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            let initialized = cx.send_request(v1::InitializeRequest::new(ProtocolVersion::V1).client_capabilities(Self::capabilities())).block_task().await?;
            let created = cx.send_request(v1::NewSessionRequest::new(cwd)).block_task().await?;
            state.lock().await.session = Some(created.session_id.clone());
            tracing::debug!(agent = ?initialized.agent_info, acp_session = %created.session_id, "ACP session ready");
            if ready.send(Opened { connection: cx.clone(), initialized, created }).is_err() { return Ok(()); }
            cx.incoming_closed().await;
            Ok(())
        }).await.map_err(|error| Error::with_source(agent.clone(), "ACP connection failed", error))?;
        Err(Error::backend(agent, "ACP connection closed"))
    }

    fn capabilities() -> v1::ClientCapabilities {
        v1::ClientCapabilities::new()
            .fs(v1::FileSystemCapabilities::new()
                .read_text_file(true)
                .write_text_file(true))
            .terminal(false)
            .meta(v1::Meta::from_iter([(
                "terminal_output".into(),
                true.into(),
            )]))
            .session(
                v1::ClientSessionCapabilities::new().config_options(
                    v1::SessionConfigOptionsCapabilities::new()
                        .boolean(v1::BooleanConfigOptionCapabilities::new()),
                ),
            )
    }
}

struct Opened {
    connection: ConnectionTo<Agent>,
    initialized: v1::InitializeResponse,
    created: v1::NewSessionResponse,
}

struct Session {
    agent: AgentId,
    connection: ConnectionTo<Agent>,
    acp_session: v1::SessionId,
    initialized: v1::InitializeResponse,
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
    fn initialization(&self) -> Option<v1::InitializeResponse> {
        Some(self.initialized.clone())
    }
    fn session_info(&self) -> Option<v1::NewSessionResponse> {
        Some(self.created.clone())
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
/// See <https://docs.rs/agent-client-protocol/2.1.0/agent_client_protocol/struct.SentRequest.html#method.on_receiving_result>.
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
