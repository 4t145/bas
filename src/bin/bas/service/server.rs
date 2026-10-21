use std::{path::PathBuf, pin::Pin, str::FromStr, sync::Arc};

use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status};

use bas::{
    acp,
    model::{
        Cursor, Owner, PromptRejection, PromptRejectionKind, PromptSubmission,
        PromptSubmissionOutcome, SessionId, TurnReceipt,
    },
};

use crate::{
    agent::AgentSpec,
    session::{AttachOptions, CreateError, CreateRequest, Registry, Session, SubmissionError},
};

use bas::service::{
    pb,
    pb::bas_server::{Bas, BasServer},
};

use super::{pagination::PageEncoder, status::metadata_error};

/// History page size used when a client does not ask for one.
const DEFAULT_PAGE: usize = 128;

/// Implements `bas.v1.Bas` on top of a [`Registry`].
#[derive(Clone)]
pub struct Service {
    registry: Arc<Registry>,
    manager: Option<Arc<crate::management::Manager>>,
}

impl Service {
    /// Serves the sessions held by `registry`.
    #[must_use]
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            manager: None,
        }
    }

    pub fn with_manager(mut self, manager: Arc<crate::management::Manager>) -> Self {
        self.manager = Some(manager);
        self
    }

    fn manager(&self) -> Result<Arc<crate::management::Manager>, Status> {
        self.manager
            .clone()
            .ok_or_else(|| Status::unimplemented("Agent management is unavailable"))
    }

    /// Wraps the service for `tonic`'s server builder.
    #[must_use]
    pub fn into_server(self) -> BasServer<Self> {
        BasServer::new(self)
            .send_compressed(tonic::codec::CompressionEncoding::Gzip)
            .max_decoding_message_size(bas::service::MAX_MESSAGE_SIZE)
            .max_encoding_message_size(bas::service::MAX_MESSAGE_SIZE)
    }

    async fn session(&self, id: &str) -> Result<Arc<Session>, Status> {
        let id =
            SessionId::from_str(id).map_err(|error| Status::invalid_argument(error.to_string()))?;

        self.registry
            .get(id)
            .await
            .map_err(|error| Status::not_found(error.to_string()))
    }
}

#[tonic::async_trait]
impl Bas for Service {
    async fn get_agent_configuration(
        &self,
        _request: Request<pb::GetAgentConfigurationRequest>,
    ) -> Result<Response<pb::AgentConfiguration>, Status> {
        Ok(Response::new(self.manager()?.configuration()?))
    }

    async fn save_agent_configuration(
        &self,
        request: Request<pb::SaveAgentConfigurationRequest>,
    ) -> Result<Response<pb::AgentConfiguration>, Status> {
        Ok(Response::new(
            self.manager()?
                .save_configuration(request.into_inner(), &self.registry)
                .await?,
        ))
    }

    type AuthenticateAgentInteractiveStream =
        Pin<Box<dyn Stream<Item = Result<pb::AuthenticationEvent, Status>> + Send + 'static>>;

    async fn authenticate_agent_interactive(
        &self,
        request: Request<tonic::Streaming<pb::AuthenticationInput>>,
    ) -> Result<Response<Self::AuthenticateAgentInteractiveStream>, Status> {
        let mut input = request.into_inner();
        let first = tokio::time::timeout(std::time::Duration::from_secs(30), input.message())
            .await
            .map_err(|_| Status::deadline_exceeded("Authentication start was not received"))??
            .ok_or_else(|| Status::invalid_argument("Authentication requires a start message"))?;
        let Some(pb::authentication_input::Kind::Start(start)) = first.kind else {
            return Err(Status::invalid_argument(
                "The first authentication message must be start",
            ));
        };
        let expected: AgentSpec =
            serde_json::from_slice(&start.expected_spec_json).map_err(|_| {
                Status::invalid_argument("expected_spec_json must contain a saved AgentSpec")
            })?;
        let catalog = self.registry.catalog();
        let spec = catalog
            .spec(&start.agent.clone().into())
            .filter(|spec| spec.as_ref() == &expected)
            .cloned()
            .ok_or_else(|| Status::failed_precondition("Agent configuration changed; retry"))?;
        if spec.kind != crate::agent::Kind::Acp {
            return Err(Status::invalid_argument(
                "Only ACP agents support authentication",
            ));
        }
        catalog
            .validate(&spec)
            .map_err(|error| Status::failed_precondition(error.to_string()))?;
        let program = crate::agent::acp::resolve_program(&spec)
            .ok_or_else(|| Status::failed_precondition("Agent executable is missing"))?;
        let managed = catalog.managed(&spec.id).cloned();
        let registry = Arc::clone(&self.registry);
        let (send, receive) = tokio::sync::mpsc::channel(32);
        let task = tokio::spawn(async move {
            let _credentials = registry.credentials_change(spec.id.clone());
            let (controls, controls_receive) = tokio::sync::mpsc::channel(16);
            let operation = crate::agent::acp::authenticate_interactive(
                &spec,
                managed.as_ref(),
                &start,
                send.clone(),
                controls_receive,
                std::time::Duration::from_secs(900),
            );
            tokio::pin!(operation);
            let result = loop {
                tokio::select! {
                    biased;
                    () = send.closed() => return,
                    () = registry.stopped() => break Err(Status::unavailable("Daemon is stopping")),
                    result = &mut operation => break result.map_err(|error| super::authentication_error::status(&error)),
                    message = input.message() => match message {
                        Ok(Some(message)) => {
                            if !Arc::ptr_eq(&catalog, &registry.catalog()) {
                                break Err(Status::failed_precondition("Agent catalog changed during authentication; retry"));
                            }
                            if controls.send(message).await.is_err() { break Err(Status::cancelled("Authentication flow closed")); }
                        }
                        Ok(None) => break Err(Status::cancelled("Authentication input closed")),
                        Err(error) => break Err(error),
                    },
                }
            };
            let result = result.and_then(|result| {
                if !Arc::ptr_eq(&catalog, &registry.catalog()) {
                    return Err(Status::failed_precondition(
                        "Agent catalog changed during authentication; retry",
                    ));
                }
                Ok(pb::AuthenticationEvent {
                    kind: Some(pb::authentication_event::Kind::Check(authentication_check(
                        result, &program,
                    ))),
                })
            });
            let _ = send.send(result).await;
        });
        let stream = futures::stream::unfold(
            (receive, tokio_util::task::AbortOnDropHandle::new(task)),
            |(mut receive, task)| async move {
                receive.recv().await.map(|event| (event, (receive, task)))
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }

    async fn list_agent_catalog(
        &self,
        request: Request<pb::ListAgentCatalogRequest>,
    ) -> Result<Response<pb::ListAgentCatalogResponse>, Status> {
        let manager = self.manager()?;
        let registrations = self.registry.catalog().registrations().cloned().collect();
        let catalog = manager
            .catalog(registrations, request.into_inner().refresh)
            .await
            .map_err(|error| Status::unavailable(error.to_string()))?;
        Ok(Response::new(pb::ListAgentCatalogResponse {
            catalog_json: serde_json::to_vec(&catalog)
                .map_err(|_| Status::internal("Could not encode agent catalog"))?,
        }))
    }

    type InstallAgentStream =
        Pin<Box<dyn Stream<Item = Result<pb::AgentOperation, Status>> + Send + 'static>>;

    async fn install_agent(
        &self,
        request: Request<pb::InstallAgentRequest>,
    ) -> Result<Response<Self::InstallAgentStream>, Status> {
        let manager = self.manager()?;
        let registry = Arc::clone(&self.registry);
        let id = request.into_inner().id;
        let (send, receive) = tokio::sync::mpsc::channel(16);
        let task = tokio::spawn(async move {
            let (progress, mut updates) = tokio::sync::mpsc::channel(16);
            let install = manager.install(&id, progress);
            tokio::pin!(install);
            let result = loop {
                tokio::select! {
                    biased;
                    () = registry.stopped() => break Err(anyhow::anyhow!("Daemon is stopping")),
                    () = send.closed() => return,
                    result = &mut install => break result,
                    Some(update) = updates.recv() => {
                        if send.send(Ok(update)).await.is_err() { return; }
                    }
                }
            };
            let result = result.and_then(|spec| {
                let store = manager
                    .store()
                    .ok_or_else(|| anyhow::anyhow!("No writable settings catalog"))?;
                registry.reload_catalog(|| Ok(store.load()?.settings.daemon.agents))?;
                Ok(spec)
            });
            let update = match result {
                Ok(spec) => serde_json::to_vec(&spec)
                    .map(|spec_json| pb::AgentOperation {
                        phase: "complete".into(),
                        message: "Agent installed and registered".into(),
                        spec_json,
                        complete: true,
                    })
                    .map_err(|_| Status::internal("Could not encode installed agent")),
                Err(error) => Err(Status::failed_precondition(error.to_string())),
            };
            let _ = send.send(update).await;
        });
        let stream = futures::stream::unfold(
            (receive, tokio_util::task::AbortOnDropHandle::new(task)),
            |(mut receive, task)| async move {
                receive.recv().await.map(|update| (update, (receive, task)))
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }

    async fn remove_agent(
        &self,
        request: Request<pb::RemoveAgentRequest>,
    ) -> Result<Response<pb::RemoveAgentResponse>, Status> {
        let manager = self.manager()?;
        manager
            .remove(&request.into_inner().id)
            .await
            .map_err(|error| Status::failed_precondition(error.to_string()))?;
        self.registry
            .reload_catalog(|| {
                Ok(manager
                    .store()
                    .ok_or_else(|| anyhow::anyhow!("No writable settings catalog"))?
                    .load()?
                    .settings
                    .daemon
                    .agents)
            })
            .map_err(|error| Status::internal(error.to_string()))?;
        Ok(Response::new(pb::RemoveAgentResponse {}))
    }

    async fn logout_agent(
        &self,
        request: Request<pb::LogoutAgentRequest>,
    ) -> Result<Response<pb::LogoutAgentResponse>, Status> {
        let request = request.into_inner();
        let expected: AgentSpec =
            serde_json::from_slice(&request.expected_spec_json).map_err(|_| {
                Status::invalid_argument("expected_spec_json must contain a saved AgentSpec")
            })?;
        let catalog = self.registry.catalog();
        let spec = catalog
            .spec(&request.agent.into())
            .filter(|spec| spec.as_ref() == &expected)
            .ok_or_else(|| Status::failed_precondition("Agent configuration changed; retry"))?;
        if spec.kind != crate::agent::Kind::Acp {
            return Err(Status::invalid_argument("Only ACP agents support logout"));
        }
        catalog
            .validate(spec)
            .map_err(|error| Status::failed_precondition(error.to_string()))?;
        let _credentials = self.registry.credentials_change(spec.id.clone());
        tokio::select! {
            result = tokio::time::timeout(self.registry.startup_timeout(), crate::agent::acp::logout(spec)) => {
                result.map_err(|_| Status::deadline_exceeded("Logout timed out; check authentication state before retrying"))?
                    .map_err(CreateError::Agent)?;
            }
            () = self.registry.stopped() => return Err(Status::unavailable("Daemon is stopping")),
        }
        Ok(Response::new(pb::LogoutAgentResponse {}))
    }

    async fn authenticate_agent(
        &self,
        request: Request<pb::AuthenticateAgentRequest>,
    ) -> Result<Response<pb::AuthenticateAgentResponse>, Status> {
        let request = request.into_inner();
        let expected: AgentSpec =
            serde_json::from_slice(&request.expected_spec_json).map_err(|_| {
                Status::invalid_argument("expected_spec_json must contain a saved AgentSpec")
            })?;
        let catalog = self.registry.catalog();
        let spec = catalog
            .spec(&request.agent.into())
            .filter(|spec| spec.as_ref() == &expected)
            .ok_or_else(|| Status::failed_precondition("Agent configuration changed; retry"))?;
        if spec.kind != crate::agent::Kind::Acp {
            return Err(Status::invalid_argument(
                "Only ACP agents support authentication",
            ));
        }
        catalog
            .validate(spec)
            .map_err(|error| Status::failed_precondition(error.to_string()))?;
        let program = crate::agent::acp::resolve_program(spec)
            .ok_or_else(|| Status::failed_precondition("Agent executable is missing"))?;
        let expected_method = if request.expected_method_json.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(&request.expected_method_json).map_err(|_| {
                    Status::invalid_argument(
                        "expected_method_json must contain a method descriptor",
                    )
                })?,
            )
        };
        let _credentials = self.registry.credentials_change(spec.id.clone());
        let result = tokio::select! {
            result = crate::agent::acp::authenticate_expected(spec, &request.method_id, expected_method, std::time::Duration::from_secs(300)) => result,
            () = self.registry.stopped() => return Err(Status::unavailable("Daemon is stopping")),
        }.map_err(|error| super::authentication_error::status(&error))?;
        if !Arc::ptr_eq(&catalog, &self.registry.catalog()) {
            return Err(Status::failed_precondition(
                "Agent catalog changed during authentication; retry",
            ));
        }
        let (status, initialized, authentication_error) = match result {
            crate::agent::acp::CheckResult::Connected(initialized) => {
                (pb::AgentCheckStatus::Connected, initialized, None)
            }
            crate::agent::acp::CheckResult::AuthenticationRequired(initialized, error) => (
                pb::AgentCheckStatus::AuthenticationRequired,
                initialized,
                Some((&error).into()),
            ),
        };
        Ok(Response::new(pb::AuthenticateAgentResponse {
            check: Some(pb::CheckAgentResponse {
                status: status.into(),
                message: "Authentication flow completed; session readiness checked".into(),
                resolved_program: program.to_string_lossy().into_owned(),
                initialization: Some((&initialized).into()),
                authentication_error,
            }),
        }))
    }

    async fn list_agents(
        &self,
        _request: Request<pb::ListAgentsRequest>,
    ) -> Result<Response<pb::ListAgentsResponse>, Status> {
        let agents = self
            .registry
            .catalog()
            .specs()
            .filter(|spec| {
                spec.kind == crate::agent::Kind::Embedded
                    || crate::agent::acp::resolve_program(spec).is_some()
            })
            .map(|spec| pb::Agent {
                id: spec.id.to_string(),
                kind: spec.kind.to_string(),
                command: spec.command().map(ToString::to_string).unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(pb::ListAgentsResponse { agents }))
    }

    async fn check_agent(
        &self,
        request: Request<pb::CheckAgentRequest>,
    ) -> Result<Response<pb::CheckAgentResponse>, Status> {
        use crate::agent::{Kind, acp};
        use pb::AgentCheckStatus as Check;
        let request = request.into_inner();
        let expected: AgentSpec =
            serde_json::from_slice(&request.expected_spec_json).map_err(|_| {
                Status::invalid_argument("expected_spec_json must contain a saved AgentSpec")
            })?;
        let catalog = self.registry.catalog();
        let id = request.agent.into();
        let spec = catalog
            .spec(&id)
            .filter(|spec| spec.as_ref() == &expected)
            .ok_or_else(|| {
                Status::failed_precondition("Agent configuration changed; save and retry")
            })?;
        catalog
            .validate(spec)
            .map_err(|error| Status::failed_precondition(error.to_string()))?;
        let mut response = pb::CheckAgentResponse {
            status: Check::Connected.into(),
            message: "Embedded agent is ready".into(),
            resolved_program: String::new(),
            initialization: None,
            authentication_error: None,
        };
        if spec.kind == Kind::Acp {
            if let Some(program) = acp::resolve_program(spec) {
                response.resolved_program = program.to_string_lossy().into_owned();
                response.status = Check::Installed.into();
                response.message =
                    "Executable is installed; connection and login have not been checked".into();
                if request.connect {
                    let result = tokio::select! {
                        result = acp::check_connection(spec, self.registry.startup_timeout()) => result,
                        () = self.registry.stopped() => return Err(Status::unavailable("Daemon is stopping")),
                    };
                    match result {
                        Ok(acp::CheckResult::Connected(initialized)) => {
                            response.status = Check::Connected.into();
                            response.message = "ACP initialize and newSession succeeded".into();
                            response.initialization = Some((&initialized).into());
                        }
                        Ok(acp::CheckResult::AuthenticationRequired(initialized, error)) => {
                            response.status = Check::AuthenticationRequired.into();
                            response.message = "Agent authentication required".into();
                            response.initialization = Some((&initialized).into());
                            response.authentication_error = Some((&error).into());
                        }
                        Err(message) => {
                            response.status = Check::Failed.into();
                            response.message = message.into();
                        }
                    }
                }
            } else {
                response.status = Check::MissingExecutable.into();
                response.message =
                    "Executable is missing or not executable on the daemon host".into();
            }
        }
        // A replaced catalog invalidates even an otherwise successful probe.
        if !Arc::ptr_eq(&catalog, &self.registry.catalog()) {
            return Err(Status::failed_precondition(
                "Agent catalog changed during check; retry",
            ));
        }
        Ok(Response::new(response))
    }

    async fn create_session(
        &self,
        request: Request<pb::CreateSessionRequest>,
    ) -> Result<Response<pb::CreateSessionResponse>, Status> {
        let request = request.into_inner();
        let current = Owner::current();
        let agent = if request.agent.is_empty() {
            AgentSpec::DEFAULT_ID.into()
        } else {
            request.agent.into()
        };

        let session = self
            .registry
            .create(CreateRequest {
                agent,
                owner: Owner {
                    user: request.owner_user.unwrap_or(current.user),
                    host: request.owner_host.unwrap_or(current.host),
                },
                cwd: request.cwd.map(PathBuf::from),
                title: request.title,
            })
            .await?;

        Ok(Response::new(pb::CreateSessionResponse {
            session: Some(pb::Session::try_from(&session.snapshot()).map_err(metadata_error)?),
        }))
    }

    async fn list_agent_sessions(
        &self,
        request: Request<pb::ListAgentSessionsRequest>,
    ) -> Result<Response<pb::ListAgentSessionsResponse>, Status> {
        let request = request.into_inner();
        let cwd = request.cwd.map(PathBuf::from);
        if cwd.as_ref().is_some_and(|cwd| !cwd.is_absolute()) {
            return Err(Status::invalid_argument(
                "Agent history cwd must be absolute",
            ));
        }
        let agent: bas::model::AgentId = request.agent.into();
        let listing = self
            .registry
            .list_agent_sessions(agent.clone(), cwd)
            .await?;
        Ok(Response::new(pb::ListAgentSessionsResponse {
            sessions: Some((&listing.sessions).into()),
            initialization: Some((&listing.initialization).into()),
            imported_sessions: self.registry.imported_sessions(&agent).await,
        }))
    }

    async fn import_agent_session(
        &self,
        request: Request<pb::ImportAgentSessionRequest>,
    ) -> Result<Response<pb::CreateSessionResponse>, Status> {
        let request = request.into_inner();
        let info: acp::SessionInfo = request
            .session_info
            .ok_or_else(|| Status::invalid_argument("Agent SessionInfo is required"))?
            .decode()
            .map_err(metadata_error)?;
        if !info.cwd.is_absolute() {
            return Err(Status::invalid_argument("Agent workspace must be absolute"));
        }
        if !info.additional_directories.is_empty() {
            return Err(Status::failed_precondition(
                "Importing additional workspace roots is not supported yet",
            ));
        }
        let session = self
            .registry
            .import_agent_session(
                CreateRequest {
                    agent: request.agent.into(),
                    owner: Owner::current(),
                    cwd: Some(info.cwd),
                    title: info.title,
                },
                info.session_id.to_string(),
            )
            .await?;
        Ok(Response::new(pb::CreateSessionResponse {
            session: Some(pb::Session::try_from(&session.snapshot()).map_err(metadata_error)?),
        }))
    }

    async fn delete_agent_session(
        &self,
        request: Request<pb::DeleteAgentSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        let request = request.into_inner();
        if request.session_id.is_empty() {
            return Err(Status::invalid_argument("Agent session ID is required"));
        }
        self.registry
            .delete_agent_session(request.agent.into(), request.session_id)
            .await?;
        Ok(Response::new(pb::DeleteSessionResponse {}))
    }

    async fn close_session(
        &self,
        request: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        let session = self.session(&request.into_inner().session).await?;
        if session.meta().status != bas::model::SessionStatus::Stopped {
            session.close_backend().await?;
            self.registry.permissions().close(session.id());
            self.registry.elicitations().close(session.id());
        }
        Ok(Response::new(pb::GetSessionResponse {
            session: Some(pb::Session::try_from(&session.snapshot()).map_err(metadata_error)?),
        }))
    }

    async fn restart_session(
        &self,
        request: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        let session = self.session(&request.into_inner().session).await?;
        self.registry.restart_session(&session).await?;
        Ok(Response::new(pb::GetSessionResponse {
            session: Some(pb::Session::try_from(&session.snapshot()).map_err(metadata_error)?),
        }))
    }

    async fn list_sessions(
        &self,
        request: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        let include_archived = request.into_inner().include_archived;
        let sessions = self
            .registry
            .snapshots()
            .await
            .iter()
            .filter(|session| include_archived || !session.meta.archived)
            .map(pb::Session::try_from)
            .collect::<Result<_, _>>()
            .map_err(metadata_error)?;

        Ok(Response::new(pb::ListSessionsResponse { sessions }))
    }

    async fn get_session(
        &self,
        request: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        let session = self.session(&request.into_inner().session).await?;

        Ok(Response::new(pb::GetSessionResponse {
            session: Some(pb::Session::try_from(&session.snapshot()).map_err(metadata_error)?),
        }))
    }

    async fn delete_session(
        &self,
        request: Request<pb::DeleteSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        let id = request.into_inner().session;
        let id = SessionId::from_str(&id)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;

        self.registry.remove(id).await?;

        Ok(Response::new(pb::DeleteSessionResponse {}))
    }

    type AttachStream = Pin<Box<dyn Stream<Item = Result<pb::Event, Status>> + Send + 'static>>;

    async fn attach(
        &self,
        request: Request<pb::AttachRequest>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let attachment = session
            .try_attach(AttachOptions {
                replay: request.replay as usize,
                cursor: request.cursor.map(Cursor::from_raw),
            })
            .await?;
        let stream = attachment.into_stream().map(|event| match event {
            Ok(event) => Ok(pb::Event::from(&event)),
            Err(error) => Err(error.into()),
        });
        Ok(Response::new(Box::pin(stream)))
    }

    type SetForegroundStream =
        Pin<Box<dyn Stream<Item = Result<pb::ForegroundResponse, Status>> + Send + 'static>>;

    async fn set_foreground(
        &self,
        request: Request<tonic::Streaming<pb::ForegroundRequest>>,
    ) -> Result<Response<Self::SetForegroundStream>, Status> {
        let service = self.clone();
        let stream = futures::stream::try_unfold(
            (request.into_inner(), None),
            move |(mut requests, mut foreground)| {
                let service = service.clone();
                async move {
                    let request = tokio::select! {
                        biased;
                        () = service.registry.stopped() => return Ok(None),
                        request = requests.message() => request?,
                    };
                    let Some(request) = request else {
                        return Ok(None);
                    };
                    foreground.take();
                    foreground = if let Some(session) = request.session {
                        Some(service.session(&session).await?.foreground())
                    } else {
                        None
                    };
                    Ok(Some((pb::ForegroundResponse {}, (requests, foreground))))
                }
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }

    async fn prompt(
        &self,
        request: Request<pb::PromptSubmission>,
    ) -> Result<Response<pb::PromptSubmissionResponse>, Status> {
        let submission = match PromptSubmission::try_from(request.into_inner()) {
            Ok(submission) => submission,
            Err(error) => {
                return submission_response(Err(PromptRejection {
                    kind: PromptRejectionKind::InvalidRequest,
                    message: error.to_string(),
                }
                .into()));
            }
        };
        let session = match self
            .session(&submission.request.session_id.to_string())
            .await
        {
            Ok(session) => session,
            Err(error) => {
                return submission_response(Err(PromptRejection {
                    kind: if error.code() == tonic::Code::InvalidArgument {
                        PromptRejectionKind::InvalidRequest
                    } else {
                        PromptRejectionKind::Unavailable
                    },
                    message: error.message().to_owned(),
                }
                .into()));
            }
        };
        match session.known_submission(&submission).await {
            Ok(Some(receipt)) => return submission_response(Ok(receipt)),
            Ok(None) => {}
            Err(error) => return submission_response(Err(error)),
        }
        if session.meta().archived {
            return submission_response(session.accept_prompt(submission).await);
        }
        let _backend = match self.registry.use_backend(&session).await {
            Ok(lease) => lease,
            Err(error) => {
                // Another caller may have registered this identity while restoration
                // was pending. An accepted submission remains queryable when inactive.
                return match session.known_submission(&submission).await {
                    Ok(Some(receipt)) => submission_response(Ok(receipt)),
                    Err(error) => submission_response(Err(error)),
                    Ok(None)
                        if matches!(error, CreateError::Storage(_) | CreateError::Session(_)) =>
                    {
                        Err(error.into())
                    }
                    Ok(None) => submission_response(Err(PromptRejection {
                        kind: PromptRejectionKind::Unavailable,
                        message: Status::from(error).message().to_owned(),
                    }
                    .into())),
                };
            }
        };
        let result = session.accept_prompt(submission).await;
        if result.is_ok() {
            self.registry.record_agent_use(session.meta().agent);
        }
        submission_response(result)
    }

    async fn get_prompt_acceptance(
        &self,
        request: Request<pb::PromptAcceptanceRequest>,
    ) -> Result<Response<pb::PromptAcceptanceResponse>, Status> {
        let request = request.into_inner();
        let turn = request
            .turn
            .parse()
            .map_err(|error: bas::model::ParseIdError| {
                Status::invalid_argument(error.to_string())
            })?;
        let session = self.session(&request.session).await?;
        let accepted = session.prompt_acceptance(turn).await?;
        Ok(Response::new(pb::PromptAcceptanceResponse {
            accepted: accepted.map(Into::into),
        }))
    }

    async fn cancel(
        &self,
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::CancelResponse>, Status> {
        let request: acp::CancelNotification = decode(request)?;
        let session = self.session(&request.session_id.to_string()).await?;
        if session.meta().status == bas::model::SessionStatus::Stopped {
            return Ok(Response::new(pb::CancelResponse {}));
        }
        let _backend = self.registry.use_backend(&session).await?;
        session.cancel_acp(request).await?;
        self.registry.permissions().cancel(session.id());
        self.registry.elicitations().cancel(session.id());

        Ok(Response::new(pb::CancelResponse {}))
    }

    type WatchPermissionsStream =
        Pin<Box<dyn Stream<Item = Result<pb::PermissionSnapshot, Status>> + Send + 'static>>;

    async fn watch_permissions(
        &self,
        request: Request<pb::WatchPermissionsRequest>,
    ) -> Result<Response<Self::WatchPermissionsStream>, Status> {
        let session = self.session(&request.into_inner().session).await?;
        let id = session.id();
        let subscription = self.registry.permissions().subscribe(id);
        // Deletion can race subscription setup after the first registry lookup.
        if let Err(error) = self.registry.get(id).await {
            self.registry.permissions().close(id);
            return Err(Status::not_found(error.to_string()));
        }
        let registry = Arc::clone(&self.registry);
        let stream =
            futures::stream::try_unfold((subscription, true), move |(mut subscription, first)| {
                let registry = Arc::clone(&registry);
                async move {
                    if !first {
                        tokio::select! {
                            biased;
                            () = registry.stopped() => return Ok(None),
                            result = subscription.snapshots.changed() => {
                                if result.is_err() { return Ok(None); }
                            }
                        }
                    }
                    let pending = subscription
                        .snapshots
                        .borrow_and_update()
                        .iter()
                        .map(pb::PendingPermission::from)
                        .collect();
                    Ok(Some((
                        pb::PermissionSnapshot {
                            session: id.to_string(),
                            pending,
                        },
                        (subscription, false),
                    )))
                }
            });
        Ok(Response::new(Box::pin(stream)))
    }

    async fn respond_permission(
        &self,
        request: Request<pb::RespondPermissionRequest>,
    ) -> Result<Response<pb::RespondPermissionResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let response: acp::RequestPermissionResponse = request
            .response
            .ok_or_else(|| Status::invalid_argument("missing ACP permission response"))?
            .decode()
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        self.registry
            .permissions()
            .respond(session.id(), &request.request_id, response.outcome)
            .map_err(|error| match error {
                crate::agent::permissions::PermissionError::InvalidOption => {
                    Status::invalid_argument(error.to_string())
                }
                crate::agent::permissions::PermissionError::Expired => {
                    Status::failed_precondition(error.to_string())
                }
            })?;
        Ok(Response::new(pb::RespondPermissionResponse {}))
    }

    type WatchElicitationsStream =
        Pin<Box<dyn Stream<Item = Result<pb::ElicitationSnapshot, Status>> + Send + 'static>>;

    async fn watch_elicitations(
        &self,
        request: Request<pb::WatchElicitationsRequest>,
    ) -> Result<Response<Self::WatchElicitationsStream>, Status> {
        let session = self.session(&request.into_inner().session).await?;
        let id = session.id();
        let subscription = self.registry.elicitations().subscribe(id);
        if let Err(error) = self.registry.get(id).await {
            self.registry.elicitations().close(id);
            return Err(Status::not_found(error.to_string()));
        }
        let registry = Arc::clone(&self.registry);
        let stream = futures::stream::try_unfold(
            (subscription, true),
            move |(mut subscription, first)| {
                let registry = Arc::clone(&registry);
                async move {
                    if !first {
                        tokio::select! {
                            biased;
                            () = registry.stopped() => return Ok(None),
                            result = subscription.snapshots.changed() => if result.is_err() { return Ok(None); },
                        }
                    }
                    let snapshot = subscription.snapshots.borrow_and_update().clone();
                    Ok(Some((
                        pb::ElicitationSnapshot {
                            session: id.to_string(),
                            pending: snapshot
                                .pending
                                .iter()
                                .map(|pending| pb::PendingElicitation {
                                    request_id: pending.request_id.clone(),
                                    request: Some((&pending.request).into()),
                                    answered: pending.answered,
                                })
                                .collect(),
                            completed_ids: snapshot.completed_ids,
                        },
                        (subscription, false),
                    )))
                }
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }

    async fn respond_elicitation(
        &self,
        request: Request<pb::RespondElicitationRequest>,
    ) -> Result<Response<pb::RespondElicitationResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let response = request
            .response
            .ok_or_else(|| Status::invalid_argument("Missing elicitation response"))?
            .decode()
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        self.registry
            .elicitations()
            .respond(session.id(), &request.request_id, response)
            .map_err(|error| match error {
                crate::agent::elicitation::ElicitationError::Invalid(_) => {
                    Status::invalid_argument(error.to_string())
                }
                crate::agent::elicitation::ElicitationError::Expired => {
                    Status::failed_precondition(error.to_string())
                }
            })?;
        Ok(Response::new(pb::RespondElicitationResponse {}))
    }

    async fn set_config_option(
        &self,
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::AcpPayload>, Status> {
        let request: acp::SetSessionConfigOptionRequest = decode(request)?;
        let session = self.session(&request.session_id.to_string()).await?;
        let _backend = self.registry.use_backend(&session).await?;
        let response = session.set_config_option(request).await?;
        Ok(Response::new((&response).into()))
    }

    async fn set_session_directories(
        &self,
        request: Request<pb::SetSessionDirectoriesRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let mut directories = Vec::new();
        for path in request.additional_directories {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(Status::invalid_argument(
                    "additional directories must be absolute",
                ));
            }
            let path = tokio::fs::canonicalize(path)
                .await
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
            bas::service::utf8_cwd(&path).map_err(|e| Status::invalid_argument(e.to_string()))?;
            if !tokio::fs::metadata(&path)
                .await
                .map_err(|e| Status::invalid_argument(e.to_string()))?
                .is_dir()
            {
                return Err(Status::invalid_argument(
                    "additional workspace must be a directory",
                ));
            }
            if session.meta().cwd.as_ref() != Some(&path) && !directories.contains(&path) {
                directories.push(path);
            }
        }
        let _backend = self.registry.use_backend(&session).await?;
        let snapshot = session
            .set_directories(directories, self.registry.startup_timeout())
            .await?;
        Ok(Response::new(pb::GetSessionResponse {
            session: Some(pb::Session::try_from(&snapshot).map_err(metadata_error)?),
        }))
    }

    async fn set_mode(
        &self,
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::AcpPayload>, Status> {
        let request: acp::SetSessionModeRequest = decode(request)?;
        let session = self.session(&request.session_id.to_string()).await?;
        let _backend = self.registry.use_backend(&session).await?;
        let response = session.set_mode(request).await?;
        Ok(Response::new((&response).into()))
    }

    async fn set_archived(
        &self,
        request: Request<pb::SetArchivedRequest>,
    ) -> Result<Response<pb::SetArchivedResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let snapshot = self
            .registry
            .set_archived(&session, request.archived)
            .await?;
        Ok(Response::new(pb::SetArchivedResponse {
            session: Some(pb::Session::try_from(&snapshot).map_err(metadata_error)?),
        }))
    }

    async fn set_title(
        &self,
        request: Request<pb::SetTitleRequest>,
    ) -> Result<Response<pb::SetTitleResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        session.set_title(&request.title).await?;

        Ok(Response::new(pb::SetTitleResponse {}))
    }

    async fn load_history(
        &self,
        request: Request<pb::LoadHistoryRequest>,
    ) -> Result<Response<pb::LoadHistoryResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let limit = if request.limit == 0 {
            DEFAULT_PAGE
        } else {
            request.limit as usize
        };
        let mut encoder = PageEncoder::new(bas::service::MAX_MESSAGE_SIZE);
        let mut result = Ok(true);
        session
            .try_visit_history(
                request.after.map(Cursor::from_raw),
                limit,
                |event, has_more| {
                    result = encoder.push(pb::Event::from(event), has_more);
                    matches!(result, Ok(true))
                },
            )
            .await?;
        result?;
        Ok(Response::new(encoder.finish()))
    }

    async fn load_history_window(
        &self,
        request: Request<pb::LoadHistoryWindowRequest>,
    ) -> Result<Response<pb::LoadHistoryWindowResponse>, Status> {
        use prost::Message as _;

        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let window = session
            .history_window(request.before.map(Cursor::from_raw), request.limit as usize)
            .await?;
        let response = pb::LoadHistoryWindowResponse {
            session: Some(pb::Session::try_from(&window.session).map_err(metadata_error)?),
            events: window.events.iter().map(pb::Event::from).collect(),
            context: window.context.iter().map(pb::Event::from).collect(),
            before: window.before,
            cursor: window.cursor,
        };
        if response.encoded_len() > bas::service::MAX_MESSAGE_SIZE {
            return Err(Status::resource_exhausted(
                "dependency-aligned history window exceeds the RPC payload limit",
            ));
        }
        Ok(Response::new(response))
    }
}

fn authentication_check(
    result: crate::agent::acp::CheckResult,
    program: &std::path::Path,
) -> pb::CheckAgentResponse {
    let (status, initialized, authentication_error) = match result {
        crate::agent::acp::CheckResult::Connected(initialized) => {
            (pb::AgentCheckStatus::Connected, initialized, None)
        }
        crate::agent::acp::CheckResult::AuthenticationRequired(initialized, error) => (
            pb::AgentCheckStatus::AuthenticationRequired,
            initialized,
            Some((&error).into()),
        ),
    };
    pb::CheckAgentResponse {
        status: status.into(),
        message: "Agent setup completed; readiness checked".into(),
        resolved_program: program.to_string_lossy().into_owned(),
        initialization: Some((&initialized).into()),
        authentication_error,
    }
}

fn submission_response(
    result: Result<TurnReceipt, SubmissionError>,
) -> Result<Response<pb::PromptSubmissionResponse>, Status> {
    let outcome = match result {
        Ok(receipt) => PromptSubmissionOutcome::Accepted(receipt),
        Err(SubmissionError::Rejected(rejection)) => PromptSubmissionOutcome::Rejected(rejection),
        Err(SubmissionError::Unknown(error)) => return Err(error.into()),
    };
    Ok(Response::new(outcome.into()))
}

fn decode<T: serde::de::DeserializeOwned>(request: Request<pb::AcpPayload>) -> Result<T, Status> {
    request
        .into_inner()
        .decode()
        .map_err(|error| Status::invalid_argument(error.to_string()))
}
