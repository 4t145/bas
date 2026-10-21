//! ACP process transport and conversion to the daemon's turn-scoped updates.

use std::{path::PathBuf, sync::Arc, time::Duration};

use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectionTo, JsonRpcMessage, on_receive_notification,
    on_receive_request,
    schema::{ProtocolVersion, v1},
};
use async_trait::async_trait;
#[cfg(test)]
use bas::model::StopReason;
use bas::model::{AgentId, SessionId, TurnId};
use futures::{FutureExt, future::BoxFuture};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    Agent as AgentTrait, Error, Host, Kind, LaunchRequest, TurnUpdate, Update,
    driver::{Driver as DriverTrait, spawn_worker},
};

mod activity;
mod authentication;
mod convert;
mod directories;
pub mod history;
mod management;
mod pool;
mod probe;
pub use authentication::{authenticate_expected, interactive as authenticate_interactive};
pub use management::logout;
pub(crate) use probe::AuthenticationError;
pub use probe::{CheckResult, check_connection, resolve_program};

#[cfg(test)]
mod tests;

const CLIENT_NAME: &str = "bas";
const BIND_DRAIN_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Default)]
pub struct Driver {
    pool: pool::Pool,
}
impl Driver {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DriverTrait for Driver {
    fn kind(&self) -> Kind {
        Kind::Acp
    }

    fn maintain_pool(&self, spec: Arc<super::AgentSpec>, target: usize, timeout: Duration) {
        self.pool.maintain(spec, target, timeout);
    }

    fn invalidate_pool(&self) {
        self.pool.invalidate();
    }

    async fn shutdown_pool(&self) {
        self.pool.shutdown().await;
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
        let warm = match self.pool.take(&request.spec) {
            Some(warm) => warm,
            None => Warm::initialize(request.spec.clone()).await?,
        };
        warm.bind(request, startup).await
    }
}

/// A process which has only initialized; it has no session or host authority.
struct Warm {
    bind: oneshot::Sender<Binding>,
    state: Arc<Mutex<State>>,
    activity: Arc<activity::Activity>,
    task: AbortOnDropHandle<()>,
}

impl Warm {
    async fn initialize(spec: Arc<super::AgentSpec>) -> Result<Self, Error> {
        let (corrupt, mut corruption) = mpsc::channel(1);
        let errors = probe::ErrorCapture::default();
        let capture = errors.clone();
        let requests = Arc::new(std::sync::Mutex::new(
            std::collections::BTreeSet::<String>::new(),
        ));
        let transport_requests = Arc::clone(&requests);
        let activity = Arc::new(activity::Activity::default());
        let transport_activity = activity.clone();
        let transport = convert::agent(spec.as_ref())?.with_debug(move |line, direction| {
            tracing::trace!(?direction, bytes = line.len(), "ACP transport line");
            transport_activity.observe(line, direction);
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
        let (bind, binding) = oneshot::channel();
        let bound = Arc::new(Context {
            bound: std::sync::OnceLock::new(),
            activity: activity.clone(),
        });
        let connection = Connection {
            transport,
            state: Arc::clone(&state),
            bound: bound.clone(),
            agent: spec.id.clone(),
            ready,
            binding,
            requests,
            errors,
        };
        let agent = spec.id.clone();
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            let panic_agent = agent.clone();
            let result = std::panic::AssertUnwindSafe(async {
                tokio::select! {
                    result = connection.run() => result,
                    Some(message) = corruption.recv() => Err(Error::backend(agent, format!("malformed ACP transport: {message}"))),
                }
            })
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(Error::backend(panic_agent, "ACP connection worker panicked")));
            if let Err(error) = result
                && let Some(bound) = bound.get()
            {
                let _ = bound.updates.send(Update::Failed(error.report())).await;
            }
        }));
        opened.await.map_err(|_| {
            Error::backend(
                spec.id.clone(),
                "ACP initialization failed; see daemon logs",
            )
        })??;
        Ok(Self {
            bind,
            state,
            activity,
            task,
        })
    }

    fn is_alive(&self) -> bool {
        !self.task.is_finished() && !self.bind.is_closed()
    }

    async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }

    async fn bind(
        self,
        request: LaunchRequest,
        startup: Startup,
    ) -> Result<Box<dyn AgentTrait>, Error> {
        let Self {
            bind,
            state,
            activity,
            task,
        } = self;
        let (ready, opened) = oneshot::channel();
        bind.send(Binding {
            session: request.session,
            cwd: request.cwd.clone(),
            additional_directories: request.additional_directories,
            host: request.host,
            updates: request.updates.clone(),
            ready,
            startup,
        })
        .map_err(|_| Error::backend(request.spec.id.clone(), "ACP warm connection closed"))?;
        // Never retry after binding: session/new may already have reached the peer.
        let opened = opened
            .await
            .map_err(|_| Error::backend(request.spec.id.clone(), "ACP session setup failed"))??;
        Ok(Box::new(Session {
            agent: request.spec.id.clone(),
            cwd: request.cwd,
            connection: opened.connection,
            acp_session: opened.created.session_id.clone(),
            initialized: opened.initialized,
            initialization_raw: opened.initialization_raw,
            created: opened.created,
            subagents: state.lock().await.subagents.clone(),
            state: state.clone(),
            activity,
            updates: request.updates,
            connection_task: Some(task),
            turn_task: None,
            cancel: CancellationToken::new(),
            cancel_request: Arc::new(std::sync::Mutex::new(None)),
            cancellation_grace: request.spec.cancellation_grace,
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            close_attempted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }))
    }
}

struct Connection {
    transport: AcpAgent,
    state: Arc<Mutex<State>>,
    bound: Arc<Context>,
    agent: AgentId,
    ready: oneshot::Sender<Result<(), Error>>,
    binding: oneshot::Receiver<Binding>,
    requests: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
    errors: probe::ErrorCapture,
}

struct Binding {
    session: SessionId,
    cwd: PathBuf,
    additional_directories: Vec<PathBuf>,
    updates: mpsc::Sender<Update>,
    host: Arc<dyn Host>,
    ready: oneshot::Sender<Result<Opened, Error>>,
    startup: Startup,
}

struct Bound {
    host: Arc<dyn Host>,
    session: SessionId,
    connection: uuid::Uuid,
    updates: mpsc::Sender<Update>,
}

struct Context {
    bound: std::sync::OnceLock<Bound>,
    activity: Arc<activity::Activity>,
}

impl Context {
    fn get(&self) -> Option<&Bound> {
        self.bound.get()
    }
}

impl Drop for Bound {
    fn drop(&mut self) {
        self.host.cancel_elicitations(self.session, self.connection);
    }
}

impl Connection {
    /// # Errors
    /// Reports initialization, transport, and connection closure failures.
    async fn run(self) -> Result<(), Error> {
        let Connection {
            transport,
            state,
            bound,
            agent,
            ready,
            binding,
            requests,
            errors,
        } = self;
        let notification_state = Arc::clone(&state);
        let notification_bound = Arc::clone(&bound);
        let permission_bound = Arc::clone(&bound);
        let permission_state = Arc::clone(&state);
        let read_bound = Arc::clone(&bound);
        let read_state = Arc::clone(&state);
        let write_bound = Arc::clone(&bound);
        let write_state = Arc::clone(&state);
        let elicitation_bound = Arc::clone(&bound);
        let completion_bound = Arc::clone(&bound);
        let elicitation_state = Arc::clone(&state);
        let startup_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let failed = Arc::clone(&startup_failed);
        let startup_agent = agent.clone();
        Client.builder().name(CLIENT_NAME)
        .on_receive_notification(async move |notification: v1::SessionNotification, _cx| {
            let Some(_activity) = notification_bound.activity.enter() else { return Ok(()); };
            let Some(bound) = notification_bound.get() else { return Ok(()); };
            notification_state.lock().await.on_notification(notification, &bound.updates).await
                .map_err(agent_client_protocol::Error::into_internal_error)
        }, on_receive_notification!())
        .on_receive_request(async move |mut request: v1::RequestPermissionRequest, responder, cx| {
            let Some(activity) = permission_bound.activity.enter() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            let Some(bound) = permission_bound.get() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            if !permission_state.lock().await.accepts_session(&request.session_id) {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            }
            request.session_id = bound.session.to_string().into();
            let host = Arc::clone(&bound.host);
            let cancellation = responder.cancellation();
            // Keep inbound dispatch free to receive cancellation and other requests
            // while the user decides. Dropping the host future clears its live card.
            cx.spawn(async move {
                let _activity = activity;
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
            let Some(_activity) = read_bound.activity.enter() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            let Some(bound) = read_bound.get() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            if !read_state.lock().await.accepts_session(&request.session_id) {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            }
            request.session_id = bound.session.to_string().into();
            let content = bound.host.read_text_file(request).await
                .map_err(file_error)?;
            responder.respond(v1::ReadTextFileResponse::new(content))?;
            Ok(())
        }, on_receive_request!())
        .on_receive_request(async move |mut request: v1::WriteTextFileRequest, responder, _cx| {
            let Some(_activity) = write_bound.activity.enter() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            let Some(bound) = write_bound.get() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            if !write_state.lock().await.accepts_session(&request.session_id) {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            }
            request.session_id = bound.session.to_string().into();
            bound.host.write_text_file(request).await
                .map_err(file_error)?;
            responder.respond(v1::WriteTextFileResponse::new())?;
            Ok(())
        }, on_receive_request!())
        .on_receive_request(async move |request: agent_client_protocol::UntypedMessage, responder, cx| {
            let Some(activity) = elicitation_bound.activity.enter() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            let Some(bound) = elicitation_bound.get() else {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            };
            if request.method != "elicitation/create" {
                responder.respond_with_error(agent_client_protocol::Error::method_not_found())?;
                return Ok(());
            }
            if let Err(error) = bas::authentication::elicitation::parse_request(&request.params) {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params().data(error.to_string()))?;
                return Ok(());
            }
            if let Some(id) = request.params.get("requestId")
                && !requests.lock().expect("ACP request IDs lock").contains(&id.to_string())
            {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            }
            if let Some(requested) = request.params.get("sessionId").and_then(serde_json::Value::as_str)
                && !elicitation_state.lock().await.accepts_session(&v1::SessionId::new(requested))
            {
                responder.respond_with_error(agent_client_protocol::Error::invalid_params())?;
                return Ok(());
            }
            let host = Arc::clone(&bound.host);
            let session = bound.session;
            let elicitation_connection = bound.connection;
            let cancellation = responder.cancellation();
            cx.spawn(async move {
                let _activity = activity;
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
            let Some(_activity) = completion_bound.activity.enter() else { return Ok(()); };
            let Some(bound) = completion_bound.get() else { return Ok(()); };
            if notification.method == "elicitation/complete"
                && let Some(id) = notification.params.get("elicitationId").and_then(serde_json::Value::as_str)
            { bound.host.complete_elicitation(bound.session, bound.connection, id); }
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
            if ready.send(Ok(())).is_err() { return Ok(()); }
            let Ok(Binding { session, cwd, additional_directories, updates, host, ready, startup }) = binding.await else {
                return Ok(());
            };
            let elicitation_connection = uuid::Uuid::new_v4();
            let mut authority = Some(Bound { host, session, connection: elicitation_connection, updates });
            while !bound.activity.try_bind(|| {
                let authority = authority.take().expect("bind authority consumed once");
                authority.host.open_elicitation_connection(session, elicitation_connection);
                let _ = bound.bound.set(authority);
            }) {
                tokio::time::sleep(BIND_DRAIN_INTERVAL).await;
            }
            if !additional_directories.is_empty()
                && initialized.agent_capabilities.session_capabilities.additional_directories.is_none()
            {
                failed.store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = ready.send(Err(Error::Unsupported { agent: startup_agent, operation: "additionalDirectories" }));
                return Ok(());
            }
            let opened_session = match startup {
                Startup::Create => {
                    let sent = cx.send_request(v1::NewSessionRequest::new(cwd).additional_directories(additional_directories));
                    let id = serde_json::to_value(sent.id())?.to_string();
                    sent.block_task().await.map_err(|error| startup_error(&errors, &id, error))
                }
                Startup::Restore(id) => {
                    // ACP requires session/load to finish history replay before its
                    // response; session/resume does not replay history.
                    // https://agentclientprotocol.com/protocol/session-setup#loading-sessions
                    state.lock().await.session = Some(id.clone());
                    if initialized.agent_capabilities.load_session {
                        let sent = cx.send_request(v1::LoadSessionRequest::new(id.clone(), cwd).additional_directories(additional_directories));
                        let request_id = serde_json::to_value(sent.id())?.to_string();
                        sent.block_task().await.map(|loaded| {
                                v1::NewSessionResponse::new(id)
                                    .modes(loaded.modes)
                                    .config_options(loaded.config_options)
                                    .meta(loaded.meta)
                            }).map_err(|error| startup_error(&errors, &request_id, error))
                    } else if initialized.agent_capabilities.session_capabilities.resume.is_some() {
                        let sent = cx.send_request(v1::ResumeSessionRequest::new(id.clone(), cwd).additional_directories(additional_directories));
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
            .subagents(v1::SubagentCapabilities::new())
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

fn file_error(error: Error) -> v1::Error {
    match error {
        Error::Acp(error) => error,
        error => v1::Error::internal_error().data(error.report()),
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
    cwd: PathBuf,
    connection: ConnectionTo<Agent>,
    acp_session: v1::SessionId,
    initialized: v1::InitializeResponse,
    initialization_raw: serde_json::Value,
    created: v1::NewSessionResponse,
    state: Arc<Mutex<State>>,
    activity: Arc<activity::Activity>,
    subagents: Arc<std::sync::Mutex<bas::model::Subagents>>,
    updates: mpsc::Sender<Update>,
    connection_task: Option<AbortOnDropHandle<()>>,
    turn_task: Option<AbortOnDropHandle<()>>,
    cancel: CancellationToken,
    cancel_request: Arc<std::sync::Mutex<Option<v1::CancelNotification>>>,
    cancellation_grace: Duration,
    closed: Arc<std::sync::atomic::AtomicBool>,
    close_attempted: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl AgentTrait for Session {
    fn try_idle_reclaim(&mut self, timeout: Duration) -> bool {
        if !self.initialized.agent_capabilities.load_session
            && self
                .initialized
                .agent_capabilities
                .session_capabilities
                .resume
                .is_none()
        {
            return false;
        }
        let Ok(state) = self.state.try_lock() else {
            return false;
        };
        if state.active.is_some() || state.reloading {
            return false;
        }
        let subagents = self.subagents.lock().expect("subagents lock");
        if subagents
            .iter()
            .any(|child| child.state.is_none() || child.is_active())
        {
            return false;
        }
        // Callback admission happens before state mutation. Holding both state
        // locks across the claim prevents an unchecked child transition.
        self.activity.try_claim(timeout)
    }

    fn subagents(&self) -> bas::model::Subagents {
        self.subagents.lock().expect("subagents lock").clone()
    }

    fn set_directories(&self, directories: Vec<PathBuf>) -> BoxFuture<'static, Result<(), Error>> {
        self.reload_directories(directories)
    }

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
        let activity = self.activity.clone();
        Box::pin(async move {
            let activity = activity
                .outbound()
                .ok_or_else(|| Error::backend("acp", "transport released"))?;
            let (done, response) = oneshot::channel();
            connection
                .send_request(request)
                .on_receiving_result(async move |result| {
                    let _activity = activity;
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
        let activity = self.activity.clone();
        Box::pin(async move {
            let activity = activity
                .outbound()
                .ok_or_else(|| Error::backend("acp", "transport released"))?;
            let (done, response) = oneshot::channel();
            connection
                .send_request(request)
                .on_receiving_result(async move |result| {
                    let _activity = activity;
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
        let activity = self
            .activity
            .outbound()
            .ok_or_else(|| Error::backend("acp", "transport released"))?;
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
            let _activity = activity;
            {
                let mut state = state.lock().await;
                state.active = Some(turn);
            }
            if cancel.is_cancelled() {
                state.lock().await.active = None;
                let _ = updates
                    .send(Update::Turn {
                        turn,
                        update: TurnUpdate::CancelledBeforeDispatch,
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
                Err(error) => Err(error),
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
        let _activity = self
            .activity
            .outbound()
            .ok_or_else(|| Error::backend("acp", "transport released"))?;
        request.session_id = self.acp_session.clone();
        *self.cancel_request.lock().expect("cancel request lock") = Some(request);
        self.cancel.cancel();
        Ok(())
    }

    fn close_session(&self) -> BoxFuture<'static, Result<(), Error>> {
        let supported = self
            .initialized
            .agent_capabilities
            .session_capabilities
            .close
            .is_some();
        let agent = self.agent.clone();
        let connection = self.connection.clone();
        let session = self.acp_session.clone();
        let closed = Arc::clone(&self.closed);
        let attempted = Arc::clone(&self.close_attempted);
        let deadline = self.cancellation_grace;
        let activity = self.activity.clone();
        Box::pin(async move {
            let _activity = activity
                .outbound()
                .ok_or_else(|| Error::backend("acp", "transport released"))?;
            if closed.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            if !supported {
                return Err(Error::Unsupported {
                    agent,
                    operation: "session/close",
                });
            }
            attempted.store(true, std::sync::atomic::Ordering::Release);
            tokio::time::timeout(
                deadline,
                connection
                    .send_request(v1::CloseSessionRequest::new(session))
                    .block_task(),
            )
            .await
            .map_err(|_| Error::backend(agent, "ACP session/close timed out"))??;
            closed.store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        })
    }

    async fn shutdown(&mut self) {
        if self
            .initialized
            .agent_capabilities
            .session_capabilities
            .close
            .is_some()
            && !self
                .close_attempted
                .load(std::sync::atomic::Ordering::Acquire)
            && let Err(error) = self.close_session().await
        {
            tracing::warn!(%error, "ACP close failed; releasing the owned process");
        }
        self.release().await;
    }

    async fn drain_updates(&mut self) {
        let (committed, receipt) = oneshot::channel();
        if self.updates.send(Update::Barrier(committed)).await.is_ok() {
            let _ = receipt.await;
        }
    }

    async fn release(&mut self) {
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
    reloading: bool,
    subagents: Arc<std::sync::Mutex<bas::model::Subagents>>,
}

impl State {
    fn accepts_session(&self, id: &v1::SessionId) -> bool {
        self.session.as_ref().is_none_or(|root| root == id)
            || self
                .subagents
                .lock()
                .expect("subagents lock")
                .contains(id.0.as_ref())
    }

    /// # Errors
    /// Rejects notifications routed to another ACP session.
    async fn on_notification(
        &mut self,
        notification: v1::SessionNotification,
        updates: &mpsc::Sender<Update>,
    ) -> Result<(), Error> {
        if !self.accepts_session(&notification.session_id) {
            return Err(Error::backend(
                "acp",
                "update references a different session",
            ));
        }
        let root = self
            .session
            .get_or_insert_with(|| notification.session_id.clone());
        let child = *root != notification.session_id;
        if let v1::SessionUpdate::SubagentUpdate(update) = &notification.update {
            if update.session_id == *root {
                return Err(Error::backend(
                    "acp",
                    "subagent cannot own the root session",
                ));
            }
            self.subagents
                .lock()
                .expect("subagents lock")
                .apply(child.then_some(notification.session_id.0.as_ref()), update)
                .map_err(|error| Error::backend("acp", error.to_string()))?;
        }
        if self.reloading {
            return Ok(());
        }
        if child {
            let _ = updates.send(Update::Subagent(notification)).await;
            return Ok(());
        }
        if matches!(
            notification.update,
            v1::SessionUpdate::SubagentUpdate(_)
                | v1::SessionUpdate::SessionMessage(_)
                | v1::SessionUpdate::SessionMessageChunk(_)
        ) {
            let _ = updates.send(Update::Acp(notification.update)).await;
            return Ok(());
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
