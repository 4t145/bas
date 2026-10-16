use std::{pin::Pin, sync::Mutex};

use futures::Stream;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status, transport::Server};

use super::*;
use crate::attachment::Attachment;
use pb::bas_server::{Bas, BasServer};
mod adapter;
mod archive;
mod permissions;
mod runtime;
mod submission;
mod tracking;

#[test]
fn native_auth_required_is_typed_but_transport_failures_stay_errors() {
    let status = Status::with_details(
        Code::Unauthenticated,
        "localized login message",
        serde_json::to_vec(&acp::Error::auth_required())
            .expect("ACP error")
            .into(),
    );
    assert!(matches!(
        creation_result(7, "tool".into(), Err(status.into())),
        Update::AuthenticationRequired { epoch: 7, agent } if agent == "tool"
    ));
    for status in [
        Status::unavailable("Authentication required"),
        Status::unauthenticated("daemon access denied, not an ACP error"),
        Status::with_details(
            Code::Unauthenticated,
            "other",
            serde_json::to_vec(&acp::Error::internal_error())
                .expect("ACP")
                .into(),
        ),
    ] {
        assert!(matches!(
            creation_result(7, "tool".into(), Err(status.into())),
            Update::Created { result: Err(_), .. }
        ));
    }
}

struct Fixture {
    client: Client,
    session: SessionId,
    turn: TurnId,
    attachments: Arc<Mutex<Vec<Option<u64>>>>,
    prompts: Arc<Mutex<Vec<acp::PromptRequest>>>,
    submissions: Arc<Mutex<Vec<PromptSubmission>>>,
    titles: Arc<Mutex<Vec<String>>>,
    permission_answers: Arc<Mutex<Vec<pb::RespondPermissionRequest>>>,
    task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Tracking(tracking::Scenario),
    Submission(submission::Scenario),
    RuntimeFailure,
    AttachFailedPrecondition,
    StreamFailedPrecondition,
    Reconnect,
    Expired,
    Missing,
    Archived,
    Approval,
    ApprovalDisconnected,
    LegacyApprovals,
}

struct Service {
    session: SessionMeta,
    runtime: SessionRuntime,
    turn: TurnId,
    history: Vec<Event>,
    attachments: Arc<Mutex<Vec<Option<u64>>>>,
    prompts: Arc<Mutex<Vec<acp::PromptRequest>>>,
    submissions: Arc<Mutex<Vec<PromptSubmission>>>,
    titles: Arc<Mutex<Vec<String>>>,
    permission_answers: Arc<Mutex<Vec<pb::RespondPermissionRequest>>>,
    permission_snapshots: tokio::sync::watch::Sender<Vec<PendingPermission>>,
    scenario: Scenario,
}

#[tonic::async_trait]
impl Bas for Service {
    async fn list_agent_sessions(
        &self,
        _: Request<pb::ListAgentSessionsRequest>,
    ) -> Result<Response<pb::ListAgentSessionsResponse>, Status> {
        Err(Status::unimplemented("unrelated history"))
    }
    async fn import_agent_session(
        &self,
        _: Request<pb::ImportAgentSessionRequest>,
    ) -> Result<Response<pb::CreateSessionResponse>, Status> {
        Err(Status::unimplemented("unrelated history"))
    }
    async fn delete_agent_session(
        &self,
        _: Request<pb::DeleteAgentSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        Err(Status::unimplemented("unrelated history"))
    }
    async fn close_session(
        &self,
        _: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        Err(Status::unimplemented("unrelated history"))
    }
    async fn restart_session(
        &self,
        _: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        Err(Status::unimplemented("unrelated history"))
    }
    type AuthenticateAgentInteractiveStream =
        Pin<Box<dyn Stream<Item = Result<pb::AuthenticationEvent, Status>> + Send>>;
    type WatchElicitationsStream =
        Pin<Box<dyn Stream<Item = Result<pb::ElicitationSnapshot, Status>> + Send>>;

    async fn authenticate_agent_interactive(
        &self,
        _: Request<tonic::Streaming<pb::AuthenticationInput>>,
    ) -> Result<Response<Self::AuthenticateAgentInteractiveStream>, Status> {
        Err(Status::unimplemented("unrelated auth"))
    }
    async fn watch_elicitations(
        &self,
        _: Request<pb::WatchElicitationsRequest>,
    ) -> Result<Response<Self::WatchElicitationsStream>, Status> {
        Err(Status::unimplemented("unrelated elicitation"))
    }
    async fn respond_elicitation(
        &self,
        _: Request<pb::RespondElicitationRequest>,
    ) -> Result<Response<pb::RespondElicitationResponse>, Status> {
        Err(Status::unimplemented("unrelated elicitation"))
    }

    type InstallAgentStream =
        Pin<Box<dyn Stream<Item = Result<pb::AgentOperation, Status>> + Send>>;
    type AttachStream = Pin<Box<dyn Stream<Item = Result<pb::Event, Status>> + Send>>;
    type SetForegroundStream =
        Pin<Box<dyn Stream<Item = Result<pb::ForegroundResponse, Status>> + Send>>;
    type WatchPermissionsStream =
        Pin<Box<dyn Stream<Item = Result<pb::PermissionSnapshot, Status>> + Send>>;

    async fn watch_permissions(
        &self,
        request: Request<pb::WatchPermissionsRequest>,
    ) -> Result<Response<Self::WatchPermissionsStream>, Status> {
        if matches!(self.scenario, Scenario::LegacyApprovals) {
            return Err(Status::unimplemented("legacy daemon"));
        }
        assert_eq!(request.into_inner().session, self.session.id.to_string());
        let session = self.session.id;
        if matches!(self.scenario, Scenario::ApprovalDisconnected) {
            let pending = self
                .permission_snapshots
                .borrow()
                .iter()
                .map(pb::PendingPermission::from)
                .collect();
            return Ok(Response::new(Box::pin(tokio_stream::iter([
                Ok(pb::PermissionSnapshot {
                    session: session.to_string(),
                    pending,
                }),
                Err(Status::unavailable("approval stream interrupted")),
            ]))));
        }
        let stream = futures::stream::unfold(
            (self.permission_snapshots.subscribe(), true),
            move |(mut snapshots, first)| async move {
                if !first && snapshots.changed().await.is_err() {
                    return None;
                }
                let pending = snapshots
                    .borrow_and_update()
                    .iter()
                    .map(pb::PendingPermission::from)
                    .collect();
                Some((
                    Ok(pb::PermissionSnapshot {
                        session: session.to_string(),
                        pending,
                    }),
                    (snapshots, false),
                ))
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }

    async fn respond_permission(
        &self,
        request: Request<pb::RespondPermissionRequest>,
    ) -> Result<Response<pb::RespondPermissionResponse>, Status> {
        let request = request.into_inner();
        assert_eq!(request.session, self.session.id.to_string());
        let response: acp::RequestPermissionResponse = request
            .response
            .as_ref()
            .expect("response payload")
            .decode()
            .expect("canonical ACP outcome");
        let mut answers = self.permission_answers.lock().expect("permission answers");
        if answers
            .iter()
            .any(|answer| answer.request_id == request.request_id)
        {
            return Err(Status::failed_precondition("already answered"));
        }
        if matches!(response.outcome, acp::RequestPermissionOutcome::Selected(selected) if selected.option_id.to_string() != "allow-once")
        {
            return Err(Status::invalid_argument("option not offered"));
        }
        answers.push(request);
        self.permission_snapshots.send_replace(Vec::new());
        Ok(Response::new(pb::RespondPermissionResponse {}))
    }

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
            session: Some(
                pb::Session::try_from(&SessionSnapshot {
                    meta: self.session.clone(),
                    runtime: self.runtime.clone(),
                })
                .expect("fixture metadata has a UTF-8 working directory"),
            ),
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
        if let Scenario::Tracking(scenario) = self.scenario {
            return tracking::attach(self, scenario, count);
        }
        if matches!(self.scenario, Scenario::Submission(_)) {
            return tracking::attach(self, tracking::Scenario::Finished, count);
        }
        if matches!(self.scenario, Scenario::Expired) && count == 1 {
            return Err(Status::data_loss("cursor evicted"));
        }
        if matches!(self.scenario, Scenario::AttachFailedPrecondition) {
            return Err(Status::failed_precondition(
                "session persistence failed: disk full",
            ));
        }
        if matches!(self.scenario, Scenario::StreamFailedPrecondition) {
            return Ok(Response::new(Box::pin(tokio_stream::iter(vec![Err(
                Status::failed_precondition("session persistence failed: disk full"),
            )]))));
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
        Ok(Response::new(pb::ListAgentsResponse { agents: Vec::new() }))
    }
    async fn check_agent(
        &self,
        _: Request<pb::CheckAgentRequest>,
    ) -> Result<Response<pb::CheckAgentResponse>, Status> {
        Err(Status::unimplemented("old daemon"))
    }
    async fn list_agent_catalog(
        &self,
        _: Request<pb::ListAgentCatalogRequest>,
    ) -> Result<Response<pb::ListAgentCatalogResponse>, Status> {
        Err(Status::unimplemented("old daemon"))
    }
    async fn install_agent(
        &self,
        _: Request<pb::InstallAgentRequest>,
    ) -> Result<Response<Self::InstallAgentStream>, Status> {
        Err(Status::unimplemented("old daemon"))
    }
    async fn remove_agent(
        &self,
        _: Request<pb::RemoveAgentRequest>,
    ) -> Result<Response<pb::RemoveAgentResponse>, Status> {
        Err(Status::unimplemented("old daemon"))
    }
    async fn authenticate_agent(
        &self,
        _: Request<pb::AuthenticateAgentRequest>,
    ) -> Result<Response<pb::AuthenticateAgentResponse>, Status> {
        Err(Status::unimplemented("old daemon"))
    }
    async fn list_sessions(
        &self,
        request: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        assert!(request.into_inner().include_archived);
        Ok(Response::new(pb::ListSessionsResponse {
            sessions: vec![
                pb::Session::try_from(&SessionSnapshot {
                    meta: self.session.clone(),
                    runtime: self.runtime.clone(),
                })
                .expect("metadata"),
            ],
        }))
    }
    async fn create_session(
        &self,
        _: Request<pb::CreateSessionRequest>,
    ) -> Result<Response<pb::CreateSessionResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn set_archived(
        &self,
        _: Request<pb::SetArchivedRequest>,
    ) -> Result<Response<pb::SetArchivedResponse>, Status> {
        Err(Status::unimplemented("old daemon"))
    }

    async fn delete_session(
        &self,
        _: Request<pb::DeleteSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }
    async fn prompt(
        &self,
        request: Request<pb::PromptSubmission>,
    ) -> Result<Response<pb::PromptSubmissionResponse>, Status> {
        let submission = PromptSubmission::try_from(request.into_inner())
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        if let Scenario::Submission(scenario) = self.scenario {
            return submission::prompt(self, scenario, submission);
        }
        self.prompts
            .lock()
            .expect("prompt log")
            .push(submission.request);
        Ok(Response::new(
            PromptSubmissionOutcome::Accepted(TurnReceipt {
                turn: submission.turn,
                cursor: Cursor::from_raw(7),
            })
            .into(),
        ))
    }
    async fn get_prompt_acceptance(
        &self,
        request: Request<pb::PromptAcceptanceRequest>,
    ) -> Result<Response<pb::PromptAcceptanceResponse>, Status> {
        if let Scenario::Submission(scenario) = self.scenario {
            return submission::acceptance(self, scenario, request.into_inner());
        }
        Ok(Response::new(pb::PromptAcceptanceResponse {
            accepted: None,
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
        request: Request<pb::SetTitleRequest>,
    ) -> Result<Response<pb::SetTitleResponse>, Status> {
        if let Scenario::Submission(scenario) = self.scenario {
            self.titles
                .lock()
                .expect("titles")
                .push(request.into_inner().title);
            if !matches!(scenario, submission::Scenario::TitleFailure) {
                return Ok(Response::new(pb::SetTitleResponse {}));
            }
        }
        Err(Status::unimplemented("not used"))
    }
}

async fn fixture(scenario: Scenario) -> Fixture {
    let mut session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    session.archived = matches!(scenario, Scenario::Archived);
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
    let submissions = Arc::new(Mutex::new(Vec::new()));
    let titles = Arc::new(Mutex::new(Vec::new()));
    let permission_answers = Arc::new(Mutex::new(Vec::new()));
    let initial_permissions = if matches!(
        scenario,
        Scenario::Approval | Scenario::ApprovalDisconnected
    ) {
        vec![PendingPermission {
            request_id: "pending-1".into(),
            request: acp::RequestPermissionRequest::new(
                id.to_string(),
                acp::ToolCallUpdate::new(
                    "call-1",
                    acp::ToolCallUpdateFields::new()
                        .title("run command")
                        .raw_input(serde_json::json!({"command": "pwd"})),
                ),
                vec![acp::PermissionOption::new(
                    "allow-once",
                    "Allow once",
                    acp::PermissionOptionKind::AllowOnce,
                )],
            ),
        }]
    } else {
        Vec::new()
    };
    let permission_snapshots = tokio::sync::watch::channel(initial_permissions).0;
    let turn = TurnId::new();
    let service = Service {
        session,
        runtime: SessionRuntime {
            failure: matches!(scenario, Scenario::RuntimeFailure).then(|| {
                RuntimeFailure::Persistence {
                    message: "disk full".into(),
                }
            }),
            ..SessionRuntime::default()
        },
        turn,
        history,
        attachments: attachments.clone(),
        prompts: prompts.clone(),
        submissions: submissions.clone(),
        titles: titles.clone(),
        permission_answers: permission_answers.clone(),
        permission_snapshots,
        scenario,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener");
    let address = listener.local_addr().expect("test address");
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(BasServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("test server");
    });
    Fixture {
        client: Client::connect_at(address).await.expect("test client"),
        session: id,
        turn,
        attachments,
        prompts,
        submissions,
        titles,
        permission_answers,
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
            submission: crate::submission::prepare(
                fixture.session,
                fixture.turn,
                &format!(
                    "分析附件{}{}",
                    attachments[0].mention, attachments[1].mention
                ),
                &attachments,
                &acp::PromptCapabilities::new()
                    .embedded_context(true)
                    .image(true),
            ),
            title: None,
        },
    )
    .await;
    assert!(matches!(
        update,
        Update::Submission {
            result: Ok(PromptSubmissionOutcome::Accepted(_)),
            ..
        }
    ));
    let update = execute(
        fixture.client.clone(),
        Command::Prompt {
            session: fixture.session,
            submission: crate::submission::prepare(
                fixture.session,
                fixture.turn,
                &format!("{}{}", attachments[0].mention, attachments[1].mention),
                &attachments,
                &acp::PromptCapabilities::new()
                    .embedded_context(true)
                    .image(true),
            ),
            title: None,
        },
    )
    .await;
    assert!(
        matches!(update, Update::Submission { result: Ok(PromptSubmissionOutcome::Accepted(receipt)), .. } if receipt.cursor.raw() == 7)
    );
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
