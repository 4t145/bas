use std::{path::PathBuf, pin::Pin, str::FromStr, sync::Arc};

use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status};

use basr::{
    acp,
    model::{Cursor, Owner, SessionId},
};

use crate::{
    agent::AgentSpec,
    session::{
        AttachError, AttachOptions, CreateError, CreateRequest, HistoryError, Registry, Session,
        SessionError,
    },
};

use basr::service::{
    pb,
    pb::basr_server::{Basr, BasrServer},
};

use super::pagination::PageEncoder;

/// History page size used when a client does not ask for one.
const DEFAULT_PAGE: usize = 128;

/// Implements `basr.v1.Basr` on top of a [`Registry`].
#[derive(Clone)]
pub struct Service {
    registry: Arc<Registry>,
}

impl Service {
    /// Serves the sessions held by `registry`.
    #[must_use]
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// Wraps the service for `tonic`'s server builder.
    #[must_use]
    pub fn into_server(self) -> BasrServer<Self> {
        BasrServer::new(self)
            .max_decoding_message_size(basr::service::MAX_MESSAGE_SIZE)
            .max_encoding_message_size(basr::service::MAX_MESSAGE_SIZE)
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
impl Basr for Service {
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
            session: Some(pb::Session::try_from(&session.meta()).map_err(metadata_error)?),
        }))
    }

    async fn list_sessions(
        &self,
        request: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        let include_archived = request.into_inner().include_archived;
        let sessions = self
            .registry
            .list()
            .await
            .iter()
            .filter(|session| include_archived || !session.archived)
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
            session: Some(pb::Session::try_from(&session.meta()).map_err(metadata_error)?),
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
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::TurnAccepted>, Status> {
        let request: acp::PromptRequest = decode(request)?;
        let session = self.session(&request.session_id.to_string()).await?;
        self.registry.ensure_restored(&session).await?;
        let receipt = session.accept_prompt(request).await?;

        Ok(Response::new(receipt.into()))
    }

    async fn cancel(
        &self,
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::CancelResponse>, Status> {
        let request: acp::CancelNotification = decode(request)?;
        let session = self.session(&request.session_id.to_string()).await?;
        self.registry.ensure_restored(&session).await?;
        session.cancel_acp(request).await?;

        Ok(Response::new(pb::CancelResponse {}))
    }

    async fn set_config_option(
        &self,
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::AcpPayload>, Status> {
        let request: acp::SetSessionConfigOptionRequest = decode(request)?;
        let session = self.session(&request.session_id.to_string()).await?;
        self.registry.ensure_restored(&session).await?;
        let response = session.set_config_option(request).await?;
        Ok(Response::new((&response).into()))
    }

    async fn set_mode(
        &self,
        request: Request<pb::AcpPayload>,
    ) -> Result<Response<pb::AcpPayload>, Status> {
        let request: acp::SetSessionModeRequest = decode(request)?;
        let session = self.session(&request.session_id.to_string()).await?;
        self.registry.ensure_restored(&session).await?;
        let response = session.set_mode(request).await?;
        Ok(Response::new((&response).into()))
    }

    async fn set_archived(
        &self,
        request: Request<pb::SetArchivedRequest>,
    ) -> Result<Response<pb::SetArchivedResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let meta = session.try_set_archived(request.archived).await?;
        Ok(Response::new(pb::SetArchivedResponse {
            session: Some(pb::Session::try_from(&meta).map_err(metadata_error)?),
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
        let mut encoder = PageEncoder::new(basr::service::MAX_MESSAGE_SIZE);
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
}

fn acp_status(error: &acp::Error) -> Status {
    let code = match error.code {
        acp::ErrorCode::AuthRequired => tonic::Code::Unauthenticated,
        acp::ErrorCode::InvalidParams | acp::ErrorCode::InvalidRequest => {
            tonic::Code::InvalidArgument
        }
        acp::ErrorCode::MethodNotFound => tonic::Code::Unimplemented,
        _ => tonic::Code::Unknown,
    };
    Status::with_details(
        code,
        error.message.clone(),
        serde_json::to_vec(error).expect("ACP error JSON").into(),
    )
}

impl From<SessionError> for Status {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Busy | SessionError::Archived | SessionError::Inactive(_) => {
                Status::failed_precondition(error.to_string())
            }
            SessionError::EmptyPrompt => Status::invalid_argument(error.to_string()),
            SessionError::History(error) => (*error).into(),
            SessionError::Storage(crate::storage::Error::Expired { .. }) => {
                Status::data_loss(error.to_string())
            }
            SessionError::Storage(
                crate::storage::Error::Future { .. } | crate::storage::Error::EmptyPage,
            ) => Status::invalid_argument(error.to_string()),
            SessionError::Storage(_) => Status::unavailable(error.to_string()),
            SessionError::CommitTask(_) => Status::unavailable(error.to_string()),
            SessionError::Agent(crate::agent::Error::Acp(error)) => {
                let code = match error.code {
                    acp::ErrorCode::InvalidParams | acp::ErrorCode::InvalidRequest => {
                        tonic::Code::InvalidArgument
                    }
                    acp::ErrorCode::MethodNotFound => tonic::Code::Unimplemented,
                    _ => tonic::Code::Unknown,
                };
                Status::with_details(
                    code,
                    error.message.clone(),
                    serde_json::to_vec(error).expect("ACP error JSON").into(),
                )
            }
            SessionError::Agent(crate::agent::Error::Unsupported { .. }) => {
                Status::unimplemented(error.to_string())
            }
            SessionError::Agent(_) => Status::unavailable(error.to_string()),
        }
    }
}

impl From<HistoryError> for Status {
    fn from(error: HistoryError) -> Self {
        match error {
            HistoryError::Expired { .. } => Status::data_loss(error.to_string()),
            HistoryError::Future { .. } | HistoryError::EmptyPage => {
                Status::invalid_argument(error.to_string())
            }
        }
    }
}

impl From<CreateError> for Status {
    fn from(error: CreateError) -> Self {
        match error {
            CreateError::UnknownAgent(_) => Status::not_found(error.to_string()),
            CreateError::NotResumable(_) => Status::failed_precondition(error.to_string()),
            CreateError::Cwd(_) => Status::invalid_argument(error.to_string()),
            CreateError::Timeout => Status::deadline_exceeded(error.to_string()),
            CreateError::Session(error) => error.into(),
            CreateError::Storage(crate::storage::Error::NotFound(_)) => {
                Status::not_found(error.to_string())
            }
            CreateError::Storage(_) => Status::unavailable(error.to_string()),
            CreateError::Agent(crate::agent::Error::Acp(acp_error)) => acp_status(&acp_error),
            CreateError::Agent(crate::agent::Error::Unsupported { .. }) => {
                Status::failed_precondition(error.to_string())
            }
            CreateError::Agent(_) | CreateError::Stopping => Status::unavailable(error.to_string()),
        }
    }
}

impl From<AttachError> for Status {
    fn from(error: AttachError) -> Self {
        match error {
            AttachError::Lagged(_) => Self::data_loss(error.to_string()),
            AttachError::Closed => Self::unavailable(error.to_string()),
            AttachError::Storage(_) => Self::unavailable(error.to_string()),
        }
    }
}

fn metadata_error(error: basr::service::Error) -> Status {
    Status::failed_precondition(error.to_string())
}

fn decode<T: serde::de::DeserializeOwned>(request: Request<pb::AcpPayload>) -> Result<T, Status> {
    request
        .into_inner()
        .decode()
        .map_err(|error| Status::invalid_argument(error.to_string()))
}
