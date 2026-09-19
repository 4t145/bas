use std::{pin::Pin, sync::Mutex};

use futures::Stream;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status, transport::Server};

use super::*;
use pb::basr_server::{Basr, BasrServer};

struct Fixture {
    client: Client,
    session: SessionId,
    attachments: Arc<Mutex<Vec<Option<u64>>>>,
    prompts: Arc<Mutex<Vec<acp::PromptRequest>>>,
    task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Reconnect,
    Expired,
    Missing,
}

struct Service {
    session: SessionMeta,
    history: Vec<Event>,
    attachments: Arc<Mutex<Vec<Option<u64>>>>,
    prompts: Arc<Mutex<Vec<acp::PromptRequest>>>,
    scenario: Scenario,
}

#[tonic::async_trait]
impl Basr for Service {
    type AttachStream = Pin<Box<dyn Stream<Item = Result<pb::Event, Status>> + Send>>;
    type SetForegroundStream =
        Pin<Box<dyn Stream<Item = Result<pb::ForegroundResponse, Status>> + Send>>;

    async fn set_foreground(
        &self,
        _: Request<tonic::Streaming<pb::ForegroundRequest>>,
    ) -> Result<Response<Self::SetForegroundStream>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn get_session(
        &self,
        _: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        Ok(Response::new(pb::GetSessionResponse {
            session: Some((&self.session).into()),
        }))
    }

    async fn load_history(
        &self,
        request: Request<pb::LoadHistoryRequest>,
    ) -> Result<Response<pb::LoadHistoryResponse>, Status> {
        if matches!(self.scenario, Scenario::Missing) {
            return Err(Status::not_found("session removed"));
        }
        let request = request.into_inner();
        let remaining: Vec<_> = self
            .history
            .iter()
            .filter(|event| event.cursor.raw() > request.after.unwrap_or(0))
            .collect();
        let events: Vec<pb::Event> = remaining
            .iter()
            .take(request.limit as usize)
            .map(|event| (*event).into())
            .collect();
        let next =
            (remaining.len() > events.len()).then(|| events.last().expect("nonempty page").cursor);
        Ok(Response::new(pb::LoadHistoryResponse { events, next }))
    }

    async fn attach(
        &self,
        request: Request<pb::AttachRequest>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let count = {
            let mut calls = self.attachments.lock().expect("attachment log");
            calls.push(request.into_inner().cursor);
            calls.len()
        };
        if matches!(self.scenario, Scenario::Expired) && count == 1 {
            return Err(Status::data_loss("cursor evicted"));
        }
        let events = if count == 1 {
            vec![
                Ok((&Event::new(
                    self.session.id,
                    Cursor::from_raw(301),
                    EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
                        acp::ContentChunk::new("partial".into()),
                    )),
                ))
                    .into()),
                Err(Status::unavailable("connection interrupted")),
            ]
        } else {
            vec![Ok((&Event::new(
                self.session.id,
                Cursor::from_raw(302),
                EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
                    acp::ContentChunk::new("complete reply".into()),
                )),
            ))
                .into())]
        };
        Ok(Response::new(Box::pin(tokio_stream::iter(events))))
    }

    async fn list_agents(
        &self,
        _: Request<pb::ListAgentsRequest>,
    ) -> Result<Response<pb::ListAgentsResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn list_sessions(
        &self,
        _: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn create_session(
        &self,
        _: Request<pb::CreateSessionRequest>,
    ) -> Result<Response<pb::CreateSessionResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn delete_session(
        &self,
        _: Request<pb::DeleteSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn prompt(
        &self,
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::TurnAccepted>, Status> {
        let prompt = request
            .into_inner()
            .decode()
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        self.prompts.lock().expect("prompt log").push(prompt);
        Ok(Response::new(pb::TurnAccepted {
            turn: uuid::Uuid::new_v4().to_string(),
        }))
    }
    async fn cancel(
        &self,
        _: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::CancelResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn set_config_option(
        &self,
        _: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::AcpPayload>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn set_mode(
        &self,
        _: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::AcpPayload>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn set_title(
        &self,
        _: Request<pb::SetTitleRequest>,
    ) -> Result<Response<pb::SetTitleResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
}

async fn fixture(scenario: Scenario) -> Fixture {
    let session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    let id = session.id;
    let history = (1..=300)
        .map(|cursor| {
            Event::new(
                id,
                Cursor::from_raw(cursor),
                EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
                    acp::ContentChunk::new(format!("message {cursor}").into()),
                )),
            )
        })
        .collect();
    let attachments = Arc::new(Mutex::new(Vec::new()));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let service = Service {
        session,
        history,
        attachments: attachments.clone(),
        prompts: prompts.clone(),
        scenario,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener");
    let address = listener.local_addr().expect("test address");
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(BasrServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("test server");
    });
    Fixture {
        client: Client::connect_at(address).await.expect("test client"),
        session: id,
        attachments,
        prompts,
        task,
    }
}

#[tokio::test]
async fn prompt_sends_text_files_and_images_as_native_acp_blocks() {
    let fixture = fixture(Scenario::Reconnect).await;
    let directory = tempfile::tempdir().expect("attachment directory");
    let text_path = directory.path().join("中文 file.txt");
    let image_path = directory.path().join("image.png");
    std::fs::write(&text_path, "附件内容\n第二行").expect("text fixture");
    std::fs::write(&image_path, b"\x89PNG\r\n\x1a\n").expect("image fixture");
    let attachments = vec![
        Attachment::load(&text_path).expect("text"),
        Attachment::load(&image_path).expect("image"),
    ];
    let expected = vec![
        "分析附件".into(),
        attachments[0].content.as_ref().clone(),
        attachments[1].content.as_ref().clone(),
    ];
    let update = execute(
        fixture.client.clone(),
        Command::Prompt {
            session: fixture.session,
            text: format!(
                "分析附件{}{}",
                attachments[0].mention, attachments[1].mention
            ),
            attachments: attachments.clone(),
            capabilities: acp::PromptCapabilities::new()
                .embedded_context(true)
                .image(true),
            title: None,
        },
    )
    .await;
    assert!(matches!(update, Update::Sent { result: Ok(()), .. }));
    let update = execute(
        fixture.client.clone(),
        Command::Prompt {
            session: fixture.session,
            text: format!("{}{}", attachments[0].mention, attachments[1].mention),
            attachments,
            capabilities: acp::PromptCapabilities::new()
                .embedded_context(true)
                .image(true),
            title: None,
        },
    )
    .await;
    assert!(matches!(update, Update::Sent { result: Ok(()), .. }));
    let prompts = fixture.prompts.lock().expect("prompt log");
    assert_eq!(prompts.len(), 2);
    assert_eq!(
        prompts[0].session_id.0.as_ref(),
        fixture.session.to_string()
    );
    assert_eq!(prompts[0].prompt, expected);
    assert_eq!(prompts[1].prompt, expected[1..]);
}

async fn collect(fixture: &Fixture) -> Vec<Update> {
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(watch(fixture.client.clone(), updates, fixture.session, 1));
    let collected = tokio::time::timeout(Duration::from_secs(10), async {
        let mut collected = Vec::new();
        while let Some(update) = events.recv().await {
            collected.push(update);
        }
        collected
    })
    .await
    .expect("watch finishes");
    task.await.expect("watch completes without panicking");
    collected
}

#[tokio::test]
async fn paginated_history_and_reconnect_resume_after_the_last_live_cursor() {
    let fixture = fixture(Scenario::Reconnect).await;
    let updates = collect(&fixture).await;
    let snapshots: Vec<_> = updates
        .iter()
        .filter_map(|update| match update {
            Update::Snapshot { events, .. } => Some(events),
            _ => None,
        })
        .collect();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].len(), 300);
    assert_eq!(
        *fixture.attachments.lock().expect("attachment log"),
        vec![Some(300), Some(301)]
    );
    let events: Vec<_> = updates
        .iter()
        .filter_map(|update| match update {
            Update::Event { event, .. } => Some(event.cursor.raw()),
            _ => None,
        })
        .collect();
    assert_eq!(events, vec![301, 302]);
}

#[tokio::test]
async fn expired_cursor_reloads_the_retained_window() {
    let fixture = fixture(Scenario::Expired).await;
    let updates = collect(&fixture).await;
    assert_eq!(
        updates
            .iter()
            .filter(|update| matches!(update, Update::Snapshot { .. }))
            .count(),
        2
    );
    assert_eq!(
        *fixture.attachments.lock().expect("attachment log"),
        vec![Some(300), Some(300)]
    );
}

#[tokio::test]
async fn missing_session_stops_retrying_and_reports_an_actionable_error() {
    let fixture = fixture(Scenario::Missing).await;
    let updates = collect(&fixture).await;
    assert!(matches!(
        updates.last(),
        Some(Update::Stream {
            state: StreamState::Unavailable(_),
            ..
        })
    ));
    assert!(
        fixture
            .attachments
            .lock()
            .expect("attachment log")
            .is_empty()
    );
}
