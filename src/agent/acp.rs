//! The Agent Client Protocol backend.
//!
//! `basrd` drives external agents (opencode, codex, gemini, …) by spawning them
//! as child processes and acting as the ACP *client*: it owns the JSON-RPC
//! connection, normalizes `session/update` notifications into [`Update`]s, and
//! answers the agent's requests for files and permissions through a
//! [`Host`](super::Host).
//!
//! The protocol reference lives at
//! <https://agentclientprotocol.com/protocol/overview>.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use agent_client_protocol::{
    AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo, on_receive_notification,
    on_receive_request,
    schema::{ProtocolVersion, v1},
};
use async_trait::async_trait;
use tokio::{
    sync::{Notify, mpsc, oneshot},
    task::JoinHandle,
};

use crate::model::{
    AgentId, ContentBlock, Delta, DeltaBlock, Message, MessageId, Role, SessionId, StopReason,
    ToolCall as ModelToolCall, ToolCallId, ToolCallStatus, ToolKind, TurnId,
};

use super::{
    Agent as AgentTrait, Command, Error, FileReadRequest, FileWriteRequest, Host, Kind,
    LaunchRequest, PermissionKind, PermissionOption, PermissionOutcome, PermissionRequest, Update,
    driver::Driver as DriverTrait,
};

/// Name reported to the agent during `initialize`.
const CLIENT_NAME: &str = "basrd";

/// Launches external agents over ACP.
#[derive(Default)]
pub struct Driver;

impl Driver {
    #[must_use]
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
        let command = request.spec.command().ok_or_else(|| {
            Error::backend(request.spec.id.clone(), "the backend has no command line")
        })?;

        let transport = transport(command, &request.spec.env);
        // ACP requires an absolute working directory.
        let cwd = absolute_cwd(request.cwd.as_deref()).map_err(|error| {
            Error::with_source(
                request.spec.id.clone(),
                "cannot resolve the working directory",
                error,
            )
        })?;
        let state = Arc::new(Mutex::new(State::default()));
        let shutdown = Arc::new(Notify::new());
        let (ready, opened) = oneshot::channel();

        let connection_task = spawn_connection(
            transport,
            cwd,
            Arc::clone(&state),
            request.updates.clone(),
            Arc::clone(&request.host),
            request.session,
            Arc::clone(&shutdown),
            ready,
        );

        let opened = match opened.await {
            Ok(opened) => opened,
            Err(_) => {
                connection_task.abort();
                return Err(Error::backend(
                    request.spec.id.clone(),
                    "the ACP agent exited before its session was ready",
                ));
            }
        };

        Ok(Box::new(Session {
            agent: request.spec.id.clone(),
            connection: opened.connection,
            acp_session: opened.session,
            state,
            updates: request.updates,
            shutdown,
            task: Mutex::new(Some(connection_task)),
        }))
    }
}

/// Runs the ACP connection until the transport closes or the daemon shuts the
/// session down.
#[allow(clippy::too_many_arguments)]
fn spawn_connection(
    transport: AcpAgent,
    cwd: PathBuf,
    state: Arc<Mutex<State>>,
    updates: mpsc::UnboundedSender<Update>,
    host: Arc<dyn Host>,
    session: SessionId,
    shutdown: Arc<Notify>,
    ready: oneshot::Sender<Opened>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let notifications = (Arc::clone(&state), updates.clone());
        let permission_host = Arc::clone(&host);
        let read_host = Arc::clone(&host);
        let write_host = Arc::clone(&host);

        let result = Client
            .builder()
            .name(CLIENT_NAME)
            .on_receive_notification(
                async move |notification: v1::SessionNotification, _cx| {
                    let (state, updates) = &notifications;
                    on_session_update(notification.update, state, updates);
                    Ok(())
                },
                on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: v1::RequestPermissionRequest, responder, _cx| {
                    let response =
                        on_permission_request(request, session, permission_host.as_ref())
                            .await
                            .map_err(agent_client_protocol::Error::into_internal_error)?;
                    responder.respond(response)?;
                    Ok(())
                },
                on_receive_request!(),
            )
            .on_receive_request(
                async move |request: v1::ReadTextFileRequest, responder, _cx| {
                    let content = read_host
                        .read_text_file(FileReadRequest {
                            session,
                            path: request.path,
                            line: request.line,
                            limit: request.limit,
                        })
                        .await
                        .map_err(agent_client_protocol::Error::into_internal_error)?;
                    responder.respond(v1::ReadTextFileResponse::new(content))?;
                    Ok(())
                },
                on_receive_request!(),
            )
            .on_receive_request(
                async move |request: v1::WriteTextFileRequest, responder, _cx| {
                    write_host
                        .write_text_file(FileWriteRequest {
                            session,
                            path: request.path,
                            content: request.content,
                        })
                        .await
                        .map_err(agent_client_protocol::Error::into_internal_error)?;
                    responder.respond(v1::WriteTextFileResponse::new())?;
                    Ok(())
                },
                on_receive_request!(),
            )
            .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
                let initialized = cx
                    .send_request(
                        v1::InitializeRequest::new(ProtocolVersion::V1)
                            .client_capabilities(capabilities()),
                    )
                    .block_task()
                    .await?;

                let created = cx
                    .send_request(v1::NewSessionRequest::new(cwd))
                    .block_task()
                    .await?;

                tracing::debug!(
                    agent = ?initialized.agent_info,
                    protocol = ?initialized.protocol_version,
                    acp_session = %created.session_id,
                    "ACP session ready"
                );

                if ready
                    .send(Opened {
                        connection: cx.clone(),
                        session: created.session_id,
                    })
                    .is_err()
                {
                    return Ok(());
                }

                tokio::select! {
                    () = cx.incoming_closed() => {}
                    () = shutdown.notified() => {}
                }
                Ok(())
            })
            .await;

        if let Err(error) = result {
            tracing::warn!(%error, "the ACP connection ended with an error");
        }
    })
}

/// Resolves the working directory ACP requires to be absolute.
fn absolute_cwd(cwd: Option<&std::path::Path>) -> std::io::Result<PathBuf> {
    match cwd {
        Some(cwd) if cwd.is_absolute() => Ok(cwd.to_path_buf()),
        Some(cwd) => Ok(std::env::current_dir()?.join(cwd)),
        None => std::env::current_dir(),
    }
}

fn transport(command: &Command, env: &BTreeMap<String, String>) -> AcpAgent {
    AcpAgent::new(
        AcpAgentConfig::new(command.program.clone())
            .args(command.args.clone())
            .envs(env.clone()),
    )
    .with_debug(|line, direction| {
        tracing::trace!(?direction, "acp: {line}");
    })
}

fn capabilities() -> v1::ClientCapabilities {
    v1::ClientCapabilities::new()
        .fs(v1::FileSystemCapabilities::new()
            .read_text_file(true)
            .write_text_file(true))
        .terminal(false)
}

struct Opened {
    connection: ConnectionTo<Agent>,
    session: v1::SessionId,
}

/// A live ACP conversation.
struct Session {
    agent: AgentId,
    connection: ConnectionTo<Agent>,
    acp_session: v1::SessionId,
    state: Arc<Mutex<State>>,
    updates: mpsc::UnboundedSender<Update>,
    shutdown: Arc<Notify>,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[async_trait]
impl AgentTrait for Session {
    async fn prompt(&self, content: Vec<ContentBlock>) -> Result<TurnId, Error> {
        let turn = TurnId::new();
        let blocks = to_acp_prompt(content);
        let connection = self.connection.clone();
        let acp_session = self.acp_session.clone();
        let updates = self.updates.clone();
        let state = Arc::clone(&self.state);

        *state.lock().expect("the ACP state mutex is never poisoned") = State::default();
        let _ = updates.send(Update::TurnStarted { turn });

        // `session/prompt` resolves only when the turn ends, but a prompt must
        // return as soon as the agent accepted it, so the turn is awaited in the
        // background and reported through the update channel.
        tokio::spawn(async move {
            match connection
                .send_request(v1::PromptRequest::new(acp_session, blocks))
                .block_task()
                .await
            {
                Ok(response) => {
                    if let Some(message) = commit(&state) {
                        let _ = updates.send(Update::Message(message));
                    }
                    let _ = updates.send(Update::TurnFinished {
                        turn,
                        stop: stop_reason(response.stop_reason),
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "the ACP prompt failed");
                    let _ = updates.send(Update::TurnFinished {
                        turn,
                        stop: StopReason::Other,
                    });
                }
            }
        });

        Ok(turn)
    }

    async fn cancel(&self) -> Result<(), Error> {
        self.connection
            .send_notification(v1::CancelNotification::new(self.acp_session.clone()))
            .map_err(|error| Error::with_source("acp", "cannot send session/cancel", error))
    }

    async fn set_title(&self, _title: &str) -> Result<(), Error> {
        // ACP only lets agents report titles (via session/update), so the daemon
        // keeps its own title and leaves the agent's untouched.
        Err(Error::Unsupported {
            agent: self.agent.clone(),
            operation: "rename a session",
        })
    }

    async fn shutdown(&self) -> Result<(), Error> {
        self.shutdown.notify_one();
        if let Some(task) = self
            .task
            .lock()
            .expect("the ACP task mutex is never poisoned")
            .take()
        {
            // The child is killed when its transport is dropped, which aborting
            // the connection task guarantees.
            task.abort();
        }
        Ok(())
    }
}

/// State the notification stream accumulates between updates.
#[derive(Default)]
struct State {
    pending: Option<PendingMessage>,
    tool_calls: HashMap<ToolCallId, ModelToolCall>,
}

/// The assistant message currently being streamed.
struct PendingMessage {
    id: MessageId,
    role: Role,
    content: Vec<ContentBlock>,
}

impl State {
    fn pending(&mut self, role: Role) -> &mut PendingMessage {
        self.pending.get_or_insert_with(|| PendingMessage {
            id: MessageId::new(),
            role,
            content: Vec::new(),
        })
    }
}

fn on_session_update(
    update: v1::SessionUpdate,
    state: &Arc<Mutex<State>>,
    updates: &mpsc::UnboundedSender<Update>,
) {
    match update {
        v1::SessionUpdate::AgentMessageChunk(chunk) => {
            stream(chunk.content, Role::Agent, state, updates);
        }
        v1::SessionUpdate::AgentThoughtChunk(chunk) => {
            stream(chunk.content, Role::Agent, state, updates);
        }
        // The daemon publishes the user turn itself, so echoing the agent's copy
        // would duplicate it.
        v1::SessionUpdate::UserMessageChunk(_) => {}
        v1::SessionUpdate::ToolCall(call) => {
            let call = tool_call(call);
            state
                .lock()
                .expect("the ACP state mutex is never poisoned")
                .tool_calls
                .insert(call.id.clone(), call.clone());
            let _ = updates.send(Update::ToolCall(call));
        }
        v1::SessionUpdate::ToolCallUpdate(update) => {
            let id = ToolCallId::new(update.tool_call_id.to_string());
            let mut state = state.lock().expect("the ACP state mutex is never poisoned");
            let merged = merge_tool_call(state.tool_calls.get(&id), update);
            state.tool_calls.insert(merged.id.clone(), merged.clone());
            drop(state);
            let _ = updates.send(Update::ToolCall(merged));
        }
        v1::SessionUpdate::SessionInfoUpdate(info) => {
            if let Some(title) = info.title.take() {
                let _ = updates.send(Update::Title(title));
            }
        }
        other => {
            // Plans, modes, usage, and config options have no representation in
            // the daemon model yet; keep them visible at trace level.
            tracing::trace!(
                update = ?std::mem::discriminant(&other),
                "ignoring an ACP session update"
            );
        }
    }
}

fn stream(
    content: v1::ContentBlock,
    role: Role,
    state: &Arc<Mutex<State>>,
    updates: &mpsc::UnboundedSender<Update>,
) {
    let v1::ContentBlock::Text(text) = content else {
        return;
    };

    let mut state = state.lock().expect("the ACP state mutex is never poisoned");
    let pending = state.pending(role);
    let block = DeltaBlock::Text {
        text: text.text.clone(),
    };
    block.clone().apply(&mut pending.content);
    let delta = Update::Delta(Delta {
        message: pending.id,
        role: pending.role,
        block,
    });
    drop(state);

    let _ = updates.send(delta);
}

fn commit(state: &Arc<Mutex<State>>) -> Option<Message> {
    let pending = state
        .lock()
        .expect("the ACP state mutex is never poisoned")
        .pending
        .take()?;

    (!pending.content.is_empty())
        .then(|| Message::with_id(pending.id, pending.role, pending.content))
}

async fn on_permission_request(
    request: v1::RequestPermissionRequest,
    session: SessionId,
    host: &dyn Host,
) -> Result<v1::RequestPermissionResponse, Error> {
    let call = tool_call_update_to_model(request.tool_call);
    let options = request
        .options
        .iter()
        .map(|option| PermissionOption {
            id: option.option_id.to_string(),
            label: option.name.clone(),
            kind: permission_kind(option.kind),
        })
        .collect();

    let outcome = host
        .request_permission(PermissionRequest {
            session,
            call,
            options,
        })
        .await?;

    Ok(v1::RequestPermissionResponse::new(match outcome {
        PermissionOutcome::Selected(option) => {
            v1::RequestPermissionOutcome::Selected(v1::SelectedPermissionOutcome::new(option.id))
        }
        PermissionOutcome::Cancelled => v1::RequestPermissionOutcome::Cancelled,
    }))
}

fn permission_kind(kind: v1::PermissionOptionKind) -> PermissionKind {
    match kind {
        v1::PermissionOptionKind::AllowOnce => PermissionKind::AllowOnce,
        v1::PermissionOptionKind::AllowAlways => PermissionKind::AllowAlways,
        v1::PermissionOptionKind::RejectOnce => PermissionKind::RejectOnce,
        v1::PermissionOptionKind::RejectAlways => PermissionKind::RejectAlways,
        _ => PermissionKind::RejectOnce,
    }
}

fn tool_call(call: v1::ToolCall) -> ModelToolCall {
    ModelToolCall {
        id: ToolCallId::new(call.tool_call_id.to_string()),
        title: call.title,
        kind: tool_kind(call.kind),
        status: tool_status(call.status),
        input: call.raw_input,
        output: call.raw_output,
        locations: location_paths(call.locations),
    }
}

fn tool_call_update_to_model(update: v1::ToolCallUpdate) -> ModelToolCall {
    let fields = update.fields;
    ModelToolCall {
        id: ToolCallId::new(update.tool_call_id.to_string()),
        title: fields.title.unwrap_or_default(),
        kind: fields.kind.map_or(ToolKind::Other, tool_kind),
        status: fields.status.map_or(ToolCallStatus::Pending, tool_status),
        input: fields.raw_input,
        output: fields.raw_output,
        locations: fields.locations.map(location_paths).unwrap_or_default(),
    }
}

fn merge_tool_call(existing: Option<&ModelToolCall>, update: v1::ToolCallUpdate) -> ModelToolCall {
    let mut merged = existing.cloned().unwrap_or_else(|| ModelToolCall {
        id: ToolCallId::new(update.tool_call_id.to_string()),
        title: String::new(),
        kind: ToolKind::Other,
        status: ToolCallStatus::Pending,
        input: None,
        output: None,
        locations: Vec::new(),
    });

    let fields = update.fields;
    if let Some(title) = fields.title {
        merged.title = title;
    }
    if let Some(kind) = fields.kind {
        merged.kind = tool_kind(kind);
    }
    if let Some(status) = fields.status {
        merged.status = tool_status(status);
    }
    if fields.raw_input.is_some() {
        merged.input = fields.raw_input;
    }
    if fields.raw_output.is_some() {
        merged.output = fields.raw_output;
    }
    if let Some(locations) = fields.locations {
        merged.locations = location_paths(locations);
    }

    merged
}

fn tool_kind(kind: v1::ToolKind) -> ToolKind {
    match kind {
        v1::ToolKind::Read => ToolKind::Read,
        v1::ToolKind::Edit => ToolKind::Edit,
        v1::ToolKind::Delete => ToolKind::Delete,
        v1::ToolKind::Move => ToolKind::Move,
        v1::ToolKind::Search => ToolKind::Search,
        v1::ToolKind::Execute => ToolKind::Execute,
        v1::ToolKind::Think => ToolKind::Think,
        v1::ToolKind::Fetch => ToolKind::Fetch,
        v1::ToolKind::SwitchMode | v1::ToolKind::Other => ToolKind::Other,
        _ => ToolKind::Other,
    }
}

fn tool_status(status: v1::ToolCallStatus) -> ToolCallStatus {
    match status {
        v1::ToolCallStatus::Pending => ToolCallStatus::Pending,
        v1::ToolCallStatus::InProgress => ToolCallStatus::InProgress,
        v1::ToolCallStatus::Completed => ToolCallStatus::Completed,
        v1::ToolCallStatus::Failed => ToolCallStatus::Failed,
        _ => ToolCallStatus::Pending,
    }
}

fn location_paths(locations: Vec<v1::ToolCallLocation>) -> Vec<String> {
    locations
        .into_iter()
        .map(|location| location.path.to_string_lossy().into_owned())
        .collect()
}

fn stop_reason(stop: v1::StopReason) -> StopReason {
    match stop {
        v1::StopReason::EndTurn => StopReason::EndTurn,
        v1::StopReason::MaxTokens | v1::StopReason::MaxTurnRequests => StopReason::MaxTokens,
        v1::StopReason::Refusal => StopReason::Refusal,
        v1::StopReason::Cancelled => StopReason::Cancelled,
        _ => StopReason::Other,
    }
}

fn to_acp_prompt(content: Vec<ContentBlock>) -> Vec<v1::ContentBlock> {
    content
        .into_iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) | ContentBlock::Thinking(text) => {
                Some(v1::ContentBlock::Text(v1::TextContent::new(text.text)))
            }
            ContentBlock::Resource(link) => Some(v1::ContentBlock::ResourceLink(
                v1::ResourceLink::new(link.name.unwrap_or_else(|| link.uri.clone()), link.uri),
            )),
            // Tool calls are agent output; they never appear in a user prompt.
            ContentBlock::ToolCall(_) => None,
        })
        .collect()
}
