//! The Tokio side of the desktop client. GPUI only exchanges owned messages.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::attachment::Attachment;
use anyhow::{Context, Result};
use basr::{
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

mod foreground;

const EVENT_CAPACITY: usize = 256;
const HISTORY_PAGE_SIZE: u32 = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT_MESSAGE: &str = "请求超时；操作可能已被服务端接收，请检查会话状态后再试。";

pub(super) struct CreateSession {
    pub epoch: u64,
    pub agent: String,
    pub title: String,
    pub cwd: String,
}

pub(super) enum Command {
    Refresh,
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
        text: String,
        attachments: Vec<Attachment>,
        capabilities: acp::PromptCapabilities,
        title: Option<String>,
    },
    Cancel(SessionId),
    SetConfig(acp::SetSessionConfigOptionRequest),
    Watch {
        session: SessionId,
        epoch: u64,
    },
    Detach,
    Foreground(Option<SessionId>),
}

pub(super) struct Catalog {
    pub agents: Vec<pb::Agent>,
    pub sessions: Vec<SessionMeta>,
}

pub(super) enum Update {
    Configured {
        session: SessionId,
        result: Result<acp::SetSessionConfigOptionResponse, String>,
    },
    Catalog(Result<Catalog, String>),
    Created {
        epoch: u64,
        result: Result<SessionMeta, String>,
    },
    AuthenticationRequired {
        epoch: u64,
        agent: String,
    },
    Renamed {
        session: SessionId,
        result: Result<(), String>,
    },
    Archived {
        session: SessionId,
        result: Result<SessionMeta, String>,
    },
    Deleted {
        session: SessionId,
        result: Result<(), String>,
    },
    Sent {
        session: SessionId,
        text: String,
        attachments: Vec<Attachment>,
        result: Result<TurnReceipt, String>,
    },
    Cancelled {
        session: SessionId,
        result: Result<(), String>,
    },
    Snapshot {
        epoch: u64,
        session: SessionMeta,
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
    Reconnecting(String),
    Closed,
    Unavailable(String),
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

    pub fn send(&self, command: Command) -> bool {
        self.commands.send(command).is_ok()
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
                        attachment = Some(tasks.spawn(watch(client.clone(), updates.clone(), session, epoch)));
                    }
                    Command::Detach => {
                        if let Some(task) = attachment.take() { task.abort(); }
                    }
                    command => {
                        let client = client.clone();
                        let updates = updates.clone();
                        let refresh = refresh.clone();
                        tasks.spawn(async move {
                            let update = execute(client, command).await;
                            let _ = updates.send(update).await;
                            refresh.notify_one();
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
                agents,
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

async fn request<T>(future: impl Future<Output = Result<T>>) -> Result<T, String> {
    match tokio::time::timeout(REQUEST_TIMEOUT, future).await {
        Ok(result) => result.map_err(|error| format!("{error:#}")),
        Err(_) => Err(REQUEST_TIMEOUT_MESSAGE.into()),
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
        .unwrap_or_else(|_| Err(tonic::Status::deadline_exceeded(REQUEST_TIMEOUT_MESSAGE)))
        .map(tonic::Response::into_inner);
    creation_result(form.epoch, form.agent, response)
}

fn creation_result(
    epoch: u64,
    agent: String,
    response: Result<pb::CreateSessionResponse, tonic::Status>,
) -> Update {
    if let Err(status) = &response
        && status.code() == Code::Unauthenticated
        && basr::service::acp_error(status)
            .is_ok_and(|error| error.code == acp::ErrorCode::AuthRequired)
    {
        return Update::AuthenticationRequired { epoch, agent };
    }
    let result = response
        .map_err(|error| error.to_string())
        .and_then(|response| {
            response
                .session
                .ok_or_else(|| "basrd 未返回创建的会话".to_owned())
        })
        .and_then(|session| {
            session
                .try_into()
                .map_err(|error: basr::service::Error| error.to_string())
        });
    Update::Created { epoch, result }
}

async fn execute(mut client: Client, command: Command) -> Update {
    match command {
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
                            anyhow::anyhow!(
                                "当前运行的 basrd 不支持归档。需要升级服务入口并重启后才能使用。\n\
                                 注意：重启会结束 Agent 并清空内存会话和历史，请先确认可丢弃。\n当前会话未删除。"
                            )
                        } else {
                            error.into()
                        }
                    })?
                    .into_inner();
                response
                    .session
                    .context("basrd 未返回归档后的会话")?
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
            text,
            attachments,
            capabilities,
            title,
        } => {
            let result = request(async {
                if let Some(title) = title {
                    client
                        .set_title(pb::SetTitleRequest {
                            session: session.to_string(),
                            title,
                        })
                        .await?;
                }
                let content = crate::attachment::prompt_content(&text, &attachments, &capabilities);
                let accepted = client
                    .prompt_acp(&acp::PromptRequest::new(session.to_string(), content))
                    .await?;
                accepted.try_into().map_err(Into::into)
            })
            .await;
            Update::Sent {
                session,
                text,
                attachments,
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
    session: SessionMeta,
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
            Some(_) => anyhow::bail!("basrd 返回了重复的历史游标"),
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
        .context("basrd 未返回会话信息")?
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
                }
                Ok(Err(error)) => {
                    if error.downcast_ref::<tonic::Status>().is_some_and(|status| {
                        matches!(status.code(), Code::NotFound | Code::InvalidArgument)
                    }) {
                        let _ = updates
                            .send(Update::Stream {
                                epoch,
                                state: StreamState::Unavailable(error.to_string()),
                            })
                            .await;
                        return;
                    }
                    if !retry(&updates, epoch, error.to_string()).await {
                        return;
                    }
                    continue;
                }
                Err(error) => {
                    if !retry(&updates, epoch, error.to_string()).await {
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
                if matches!(error.code(), Code::NotFound | Code::InvalidArgument) {
                    let _ = updates
                        .send(Update::Stream {
                            epoch,
                            state: StreamState::Unavailable(error.to_string()),
                        })
                        .await;
                    return;
                }
                if !retry(&updates, epoch, error.to_string()).await {
                    return;
                }
                continue;
            }
            Err(error) => {
                if !retry(&updates, epoch, error.to_string()).await {
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
                                    state: StreamState::Unavailable(format!(
                                        "无效的会话事件：{error}"
                                    )),
                                })
                                .await;
                            return;
                        }
                    };
                    cursor = Some(event.cursor.raw());
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
                    if error.code() == Code::DataLoss {
                        cursor = None;
                    }
                    if !retry(&updates, epoch, error.to_string()).await {
                        return;
                    }
                    break;
                }
            }
        }
    }
}

async fn retry(updates: &mpsc::Sender<Update>, epoch: u64, error: String) -> bool {
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
