//! The Tokio side of the desktop client. GPUI only exchanges owned messages.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Result;
use bas::{
    acp,
    model::*,
    service::{Client, pb},
};
use tokio::{
    runtime::Handle,
    sync::{Notify, mpsc},
    task::{AbortHandle, JoinHandle, JoinSet},
};
use tonic::{Code, transport::Endpoint};

mod elicitations;
mod failure;
mod foreground;
mod model;
mod permissions;
mod tracking;

pub(crate) use failure::{Condition, Failure, FailureContext};
pub(crate) use model::{AgentSummary, PendingElicitation};

const EVENT_CAPACITY: usize = 256;
const HISTORY_PAGE_SIZE: u32 = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub(super) struct CreateSession {
    pub epoch: u64,
    pub agent: String,
    pub title: String,
    pub cwd: String,
}

pub(super) enum Command {
    Refresh,
    AgentHistory {
        epoch: u64,
        agent: String,
        cwd: Option<std::path::PathBuf>,
    },
    ImportHistory {
        epoch: u64,
        agent: String,
        info: acp::SessionInfo,
    },
    DeleteAgentHistory {
        epoch: u64,
        agent: String,
        id: String,
    },
    CloseSession(SessionId),
    Create(CreateSession),
    Rename {
        session: SessionId,
        title: String,
    },
    Archive {
        session: SessionId,
        archived: bool,
    },
    Delete(SessionId),
    Prompt {
        session: SessionId,
        submission: Arc<PromptSubmission>,
        title: Option<String>,
    },
    ResolvePrompt {
        session: SessionId,
        submission: Arc<PromptSubmission>,
        retry: bool,
    },
    Cancel(SessionId),
    SetConfig(acp::SetSessionConfigOptionRequest),
    RespondPermission {
        session: SessionId,
        request_id: String,
        outcome: acp::RequestPermissionOutcome,
    },
    RespondElicitation {
        session: SessionId,
        request_id: String,
        response: serde_json::Value,
    },
    Watch {
        session: SessionId,
        epoch: u64,
    },
    Detach,
    Foreground(Option<SessionId>),
}

pub(super) struct Catalog {
    pub agents: Vec<AgentSummary>,
    pub sessions: Vec<SessionSnapshot>,
}

pub(super) struct AgentHistory {
    pub sessions: acp::ListSessionsResponse,
    pub initialization: acp::InitializeResponse,
    pub imported: std::collections::HashMap<String, String>,
}

pub(super) enum Update {
    AgentHistory {
        epoch: u64,
        result: Result<AgentHistory, Failure>,
    },
    ImportedHistory {
        epoch: u64,
        result: Result<SessionSnapshot, Failure>,
    },
    DeletedAgentHistory {
        epoch: u64,
        id: String,
        result: Result<(), Failure>,
    },
    ClosedSession {
        session: SessionId,
        result: Result<SessionSnapshot, Failure>,
    },
    Elicitations {
        epoch: u64,
        pending: Vec<PendingElicitation>,
        completed_ids: Vec<String>,
    },
    ElicitationUnavailable {
        epoch: u64,
        message: Failure,
    },
    ElicitationResponded {
        session: SessionId,
        request_id: String,
        result: Result<(), Failure>,
    },
    Permissions {
        epoch: u64,
        pending: Vec<PendingPermission>,
    },
    PermissionUnavailable {
        epoch: u64,
        message: Failure,
    },
    PermissionResponded {
        session: SessionId,
        request_id: String,
        result: Result<(), Failure>,
    },
    Configured {
        session: SessionId,
        result: Result<acp::SetSessionConfigOptionResponse, Failure>,
    },
    Catalog(Result<Catalog, Failure>),
    Created {
        epoch: u64,
        result: Result<SessionSnapshot, Failure>,
    },
    AuthenticationRequired {
        epoch: u64,
        agent: String,
    },
    Renamed {
        session: SessionId,
        result: Result<(), Failure>,
    },
    Archived {
        session: SessionId,
        result: Result<SessionSnapshot, Failure>,
    },
    Deleted {
        session: SessionId,
        result: Result<(), Failure>,
    },
    Submission {
        session: SessionId,
        submission: Arc<PromptSubmission>,
        result: Result<PromptSubmissionOutcome, Failure>,
    },
    TrackedTurn {
        event: Event,
    },
    TrackingFailed {
        session: SessionId,
        receipt: TurnReceipt,
        message: Failure,
    },
    Cancelled {
        session: SessionId,
        result: Result<(), Failure>,
    },
    Snapshot {
        epoch: u64,
        session: SessionSnapshot,
        events: Vec<Event>,
    },
    Event {
        epoch: u64,
        event: Event,
    },
    Stream {
        epoch: u64,
        state: StreamState,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum StreamState {
    Loading,
    Live,
    Reconnecting(Failure),
    Closed,
    Unavailable(Failure),
}

pub(super) struct Connection {
    commands: mpsc::UnboundedSender<Command>,
    task: Option<JoinHandle<()>>,
}

impl Connection {
    pub fn new(server: SocketAddr, runtime: &Handle) -> (Self, mpsc::Receiver<Update>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (updates, events) = mpsc::channel(EVENT_CAPACITY);
        let task = runtime.spawn(async move {
            let endpoint = Endpoint::from_shared(format!("http://{server}"))
                .expect("a socket address produces a valid HTTP URI")
                .connect_timeout(REQUEST_TIMEOUT);
            run(
                Client::new(endpoint.connect_lazy()).with_payload_limit(),
                receiver,
                updates,
            )
            .await;
        });
        (
            Self {
                commands,
                task: Some(task),
            },
            events,
        )
    }

    /// Returns the unqueued command so its owner can finish local operation state.
    ///
    /// # Errors
    /// Returns the original command if the connection task has stopped receiving.
    pub fn send(&self, command: Command) -> Result<(), Box<Command>> {
        self.commands
            .send(command)
            .map_err(|error| Box::new(error.0))
    }

    #[cfg(feature = "gui-tests")]
    pub fn for_test(commands: mpsc::UnboundedSender<Command>) -> Self {
        Self {
            commands,
            task: None,
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn run(
    client: Client,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: mpsc::Sender<Update>,
) {
    let refresh = Arc::new(Notify::new());
    let mut tasks = JoinSet::new();
    let (foreground, visibility) = tokio::sync::watch::channel(None);
    tasks.spawn(foreground::watch(client.clone(), visibility));
    tasks.spawn(catalog_loop(
        client.clone(),
        updates.clone(),
        refresh.clone(),
    ));
    let mut attachment: Option<AbortHandle> = None;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Refresh => refresh.notify_one(),
                    Command::Foreground(session) => { foreground.send_replace(session); }
                    Command::Watch { session, epoch } => {
                        if let Some(task) = attachment.take() { task.abort(); }
                        attachment = Some(tasks.spawn(permissions::watch_session(client.clone(), updates.clone(), session, epoch)));
                    }
                    Command::Detach => {
                        if let Some(task) = attachment.take() { task.abort(); }
                    }
                    command => {
                        let client = client.clone();
                        let updates = updates.clone();
                        let refresh = refresh.clone();
                        tasks.spawn(async move {
                            operation(client, command, updates, refresh).await;
                        });
                    }
                }
            }
            result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(error)) = result
                    && !error.is_cancelled()
                {
                    tracing::error!(%error, "GUI background task failed");
                }
            }
        }
    }
}

async fn operation(
    client: Client,
    command: Command,
    updates: mpsc::Sender<Update>,
    refresh: Arc<Notify>,
) {
    let update = execute(client.clone(), command).await;
    let accepted = match &update {
        Update::Submission {
            session,
            result: Ok(PromptSubmissionOutcome::Accepted(receipt)),
            ..
        } => Some((*session, *receipt)),
        _ => None,
    };
    if updates.send(update).await.is_err() {
        return;
    }
    refresh.notify_one();
    if let Some((session, receipt)) = accepted {
        let result = tokio::select! {
            result = tracking::track(client, session, receipt) => result,
            _ = updates.closed() => return,
        };
        let update = match result {
            Ok(event) => Update::TrackedTurn { event },
            Err(message) => Update::TrackingFailed {
                session,
                receipt,
                message,
            },
        };
        let _ = updates.send(update).await;
        refresh.notify_one();
    }
}

async fn catalog_loop(mut client: Client, updates: mpsc::Sender<Update>, refresh: Arc<Notify>) {
    loop {
        let result = request(async {
            let agents = client
                .list_agents(pb::ListAgentsRequest {})
                .await?
                .into_inner()
                .agents;
            let sessions = client
                .list_sessions(pb::ListSessionsRequest {
                    include_archived: true,
                })
                .await?
                .into_inner()
                .sessions;
            Ok(Catalog {
                agents: agents.into_iter().map(Into::into).collect(),
                sessions: sessions
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?,
            })
        })
        .await;
        if updates.send(Update::Catalog(result)).await.is_err() {
            return;
        }
        tokio::select! {
            _ = refresh.notified() => {}
            _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
        }
    }
}

/// # Errors
/// Preserves known RPC and payload failures, or returns the request deadline.
pub(crate) async fn request<T>(future: impl Future<Output = Result<T>>) -> Result<T, Failure> {
    match tokio::time::timeout(REQUEST_TIMEOUT, future).await {
        Ok(result) => result.map_err(Failure::from),
        Err(_) => Err(Failure::Timeout),
    }
}

async fn create_session(client: &mut Client, form: CreateSession) -> Update {
    let owner = Owner::current();
    let request = pb::CreateSessionRequest {
        agent: form.agent.clone(),
        title: (!form.title.trim().is_empty()).then(|| form.title.trim().to_owned()),
        cwd: (!form.cwd.trim().is_empty()).then(|| form.cwd.trim().to_owned()),
        owner_user: Some(owner.user),
        owner_host: Some(owner.host),
    };
    let response = tokio::time::timeout(REQUEST_TIMEOUT, client.create_session(request))
        .await
        .map_err(|_| Failure::Timeout.context(FailureContext::RpcDeadline))
        .and_then(|result| result.map_err(Failure::from))
        .map(tonic::Response::into_inner);
    creation_result(form.epoch, form.agent, response)
}

fn creation_result(
    epoch: u64,
    agent: String,
    response: Result<pb::CreateSessionResponse, Failure>,
) -> Update {
    if let Err(Failure::Rpc {
        code: Code::Unauthenticated,
        details,
        ..
    }) = &response
        && serde_json::from_slice::<acp::Error>(details)
            .is_ok_and(|error| error.code == acp::ErrorCode::AuthRequired)
    {
        return Update::AuthenticationRequired { epoch, agent };
    }
    let result = response
        .and_then(|response| {
            response
                .session
                .ok_or(Failure::Condition(Condition::MissingCreatedSession))
        })
        .and_then(|session| {
            session
                .try_into()
                .map_err(|error: bas::service::Error| Failure::InvalidPayload(error.to_string()))
        });
    Update::Created { epoch, result }
}

async fn execute(mut client: Client, command: Command) -> Update {
    match command {
        Command::AgentHistory { epoch, agent, cwd } => {
            let result = request(async {
                let response = client
                    .list_agent_sessions(pb::ListAgentSessionsRequest {
                        agent,
                        cwd: cwd
                            .map(|path| bas::service::utf8_cwd(&path).map(str::to_owned))
                            .transpose()?,
                    })
                    .await?
                    .into_inner();
                Ok(AgentHistory {
                    sessions: response
                        .sessions
                        .ok_or_else(|| anyhow::anyhow!("Missing Agent history"))?
                        .decode()?,
                    initialization: response
                        .initialization
                        .ok_or_else(|| anyhow::anyhow!("Missing Agent initialization"))?
                        .decode()?,
                    imported: response.imported_sessions,
                })
            })
            .await;
            Update::AgentHistory { epoch, result }
        }
        Command::ImportHistory { epoch, agent, info } => {
            let result = request(async {
                let response = client
                    .import_agent_session(pb::ImportAgentSessionRequest {
                        agent,
                        session_info: Some((&info).into()),
                    })
                    .await?
                    .into_inner();
                response
                    .session
                    .ok_or_else(|| anyhow::anyhow!("Missing imported session"))?
                    .try_into()
                    .map_err(Into::into)
            })
            .await;
            Update::ImportedHistory { epoch, result }
        }
        Command::DeleteAgentHistory { epoch, agent, id } => {
            let result = request(async {
                client
                    .delete_agent_session(pb::DeleteAgentSessionRequest {
                        agent,
                        session_id: id.clone(),
                    })
                    .await?;
                Ok(())
            })
            .await;
            Update::DeletedAgentHistory { epoch, id, result }
        }
        Command::CloseSession(session) => {
            let result = request(async {
                let response = client
                    .close_session(pb::GetSessionRequest {
                        session: session.to_string(),
                    })
                    .await?
                    .into_inner();
                response
                    .session
                    .ok_or_else(|| anyhow::anyhow!("Missing closed session"))?
                    .try_into()
                    .map_err(Into::into)
            })
            .await;
            Update::ClosedSession { session, result }
        }
        Command::RespondElicitation {
            session,
            request_id,
            response,
        } => {
            let result = request(async {
                client
                    .respond_elicitation(pb::RespondElicitationRequest {
                        session: session.to_string(),
                        request_id: request_id.clone(),
                        response: Some(pb::AcpPayload {
                            json: serde_json::to_vec(&response)?,
                        }),
                    })
                    .await?;
                Ok(())
            })
            .await;
            Update::ElicitationResponded {
                session,
                request_id,
                result,
            }
        }
        Command::RespondPermission {
            session,
            request_id,
            outcome,
        } => {
            let result = request(async {
                client
                    .respond_permission_acp(session, request_id.clone(), outcome)
                    .await?;
                Ok(())
            })
            .await;
            Update::PermissionResponded {
                session,
                request_id,
                result,
            }
        }
        Command::Create(form) => create_session(&mut client, form).await,
        Command::Rename { session, title } => {
            let result = request(async {
                client
                    .set_title(pb::SetTitleRequest {
                        session: session.to_string(),
                        title: title.clone(),
                    })
                    .await?;
                Ok(())
            })
            .await;
            Update::Renamed { session, result }
        }
        Command::Archive { session, archived } => {
            let result = request(async {
                let response = client
                    .set_archived(pb::SetArchivedRequest {
                        session: session.to_string(),
                        archived,
                    })
                    .await
                    .map_err(|error| {
                        if error.code() == Code::Unimplemented {
                            anyhow::Error::from(
                                Failure::from(error).context(FailureContext::ArchiveUnsupported),
                            )
                        } else {
                            error.into()
                        }
                    })?
                    .into_inner();
                response
                    .session
                    .ok_or(Failure::Condition(Condition::MissingArchivedSession))?
                    .try_into()
                    .map_err(Into::into)
            })
            .await;
            Update::Archived { session, result }
        }
        Command::Delete(session) => {
            let result = request(async {
                client
                    .delete_session(pb::DeleteSessionRequest {
                        session: session.to_string(),
                    })
                    .await?;
                Ok(())
            })
            .await;
            Update::Deleted { session, result }
        }
        Command::Prompt {
            session,
            submission,
            title,
        } => {
            let result = crate::submission::submit(&mut client, session, &submission, title).await;
            Update::Submission {
                session,
                submission,
                result,
            }
        }
        Command::ResolvePrompt {
            session,
            submission,
            retry,
        } => {
            let result = request(async {
                if retry {
                    Ok(client.submit_prompt(&submission).await?)
                } else {
                    client
                        .prompt_acceptance(session, submission.turn)
                        .await?
                        .map(PromptSubmissionOutcome::Accepted)
                        .ok_or_else(|| Failure::Condition(Condition::UnknownSubmission).into())
                }
            })
            .await;
            Update::Submission {
                session,
                submission,
                result,
            }
        }
        Command::SetConfig(config) => {
            let session = config
                .session_id
                .to_string()
                .parse()
                .expect("GUI uses daemon session ids");
            let result = request(async { Ok(client.set_config_option_acp(&config).await?) }).await;
            Update::Configured { session, result }
        }
        Command::Cancel(session) => {
            let result = request(async {
                client
                    .cancel_acp(&acp::CancelNotification::new(session.to_string()))
                    .await?;
                Ok(())
            })
            .await;
            Update::Cancelled { session, result }
        }
        Command::Refresh | Command::Watch { .. } | Command::Detach | Command::Foreground(_) => {
            unreachable!("control commands are handled by run")
        }
    }
}

struct Snapshot {
    session: SessionSnapshot,
    events: Vec<Event>,
}

/// # Errors
/// Fails if a history page or session metadata cannot be read or decoded.
async fn snapshot(client: &mut Client, session: SessionId) -> Result<Snapshot> {
    let mut events = Vec::new();
    let mut after = None;
    loop {
        let page = client
            .load_history(pb::LoadHistoryRequest {
                session: session.to_string(),
                after,
                limit: HISTORY_PAGE_SIZE,
            })
            .await?
            .into_inner();
        for event in page.events {
            events.push(event.try_into()?);
        }
        match page.next {
            Some(next) if Some(next) != after => after = Some(next),
            Some(_) => return Err(Failure::Condition(Condition::RepeatedHistoryCursor).into()),
            None => break,
        }
    }
    let meta = client
        .get_session(pb::GetSessionRequest {
            session: session.to_string(),
        })
        .await?
        .into_inner()
        .session
        .ok_or(Failure::Condition(Condition::MissingSessionInfo))?
        .try_into()?;
    Ok(Snapshot {
        session: meta,
        events,
    })
}

async fn watch(mut client: Client, updates: mpsc::Sender<Update>, session: SessionId, epoch: u64) {
    let mut cursor = None;
    loop {
        if cursor.is_none() {
            let _ = updates
                .send(Update::Stream {
                    epoch,
                    state: StreamState::Loading,
                })
                .await;
            match tokio::time::timeout(REQUEST_TIMEOUT, snapshot(&mut client, session)).await {
                Ok(Ok(Snapshot {
                    session: meta,
                    events,
                })) => {
                    cursor = Some(events.last().map_or(0, |event| event.cursor.raw()));
                    let failure = meta
                        .runtime
                        .failure
                        .as_ref()
                        .map(|failure| Failure::Runtime(failure.clone()));
                    if updates
                        .send(Update::Snapshot {
                            epoch,
                            session: meta,
                            events,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if let Some(message) = failure {
                        let _ = updates
                            .send(Update::Stream {
                                epoch,
                                state: StreamState::Unavailable(message),
                            })
                            .await;
                        return;
                    }
                }
                Ok(Err(error)) => {
                    if error
                        .downcast_ref::<tonic::Status>()
                        .is_some_and(|status| terminal_watch_status(status.code()))
                    {
                        let _ = updates
                            .send(Update::Stream {
                                epoch,
                                state: StreamState::Unavailable(error.into()),
                            })
                            .await;
                        return;
                    }
                    if !retry(&updates, epoch, error.into()).await {
                        return;
                    }
                    continue;
                }
                Err(_) => {
                    if !retry(
                        &updates,
                        epoch,
                        Failure::Timeout.context(FailureContext::StreamDeadline),
                    )
                    .await
                    {
                        return;
                    }
                    continue;
                }
            }
        }
        let result = tokio::time::timeout(
            REQUEST_TIMEOUT,
            client.attach(pb::AttachRequest {
                session: session.to_string(),
                cursor,
                replay: 0,
            }),
        )
        .await;
        let mut stream = match result {
            Ok(Ok(response)) => response.into_inner(),
            Ok(Err(error)) => {
                if error.code() == Code::DataLoss {
                    cursor = None;
                }
                if terminal_watch_status(error.code()) {
                    let _ = updates
                        .send(Update::Stream {
                            epoch,
                            state: StreamState::Unavailable(error.into()),
                        })
                        .await;
                    return;
                }
                if !retry(&updates, epoch, error.into()).await {
                    return;
                }
                continue;
            }
            Err(_) => {
                if !retry(
                    &updates,
                    epoch,
                    Failure::Timeout.context(FailureContext::StreamDeadline),
                )
                .await
                {
                    return;
                }
                continue;
            }
        };
        if updates
            .send(Update::Stream {
                epoch,
                state: StreamState::Live,
            })
            .await
            .is_err()
        {
            return;
        }
        loop {
            match stream.message().await {
                Ok(Some(event)) => {
                    let event: Event = match event.try_into() {
                        Ok(event) => event,
                        Err(error) => {
                            let _ = updates
                                .send(Update::Stream {
                                    epoch,
                                    state: StreamState::Unavailable(
                                        Failure::InvalidPayload(error.to_string())
                                            .context(FailureContext::InvalidEvent),
                                    ),
                                })
                                .await;
                            return;
                        }
                    };
                    if !event.is_transient() {
                        cursor = Some(event.cursor.raw());
                    }
                    if updates.send(Update::Event { epoch, event }).await.is_err() {
                        return;
                    }
                }
                Ok(None) => {
                    let _ = updates
                        .send(Update::Stream {
                            epoch,
                            state: StreamState::Closed,
                        })
                        .await;
                    return;
                }
                Err(error) => {
                    if terminal_watch_status(error.code()) {
                        let _ = updates
                            .send(Update::Stream {
                                epoch,
                                state: StreamState::Unavailable(error.into()),
                            })
                            .await;
                        return;
                    }
                    if error.code() == Code::DataLoss {
                        cursor = None;
                    }
                    if !retry(&updates, epoch, error.into()).await {
                        return;
                    }
                    break;
                }
            }
        }
    }
}

fn terminal_watch_status(code: Code) -> bool {
    matches!(
        code,
        Code::NotFound | Code::InvalidArgument | Code::FailedPrecondition
    )
}

async fn retry(updates: &mpsc::Sender<Update>, epoch: u64, error: Failure) -> bool {
    if updates
        .send(Update::Stream {
            epoch,
            state: StreamState::Reconnecting(error),
        })
        .await
        .is_err()
    {
        return false;
    }
    tokio::time::sleep(RECONNECT_DELAY).await;
    !updates.is_closed()
}

#[cfg(test)]
mod tests;
