use std::{path::PathBuf, str::FromStr, sync::Arc};

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::{
    agent::AgentSpec,
    model::{ContentBlock, Cursor, Owner, SessionId},
    session::{AttachError, AttachOptions, CreateRequest, DEFAULT_REPLAY, Registry, Session},
};

use super::{
    pb,
    pb::basr_server::{Basr, BasrServer},
};

/// Events buffered for one attached client before the daemon stops reading.
const STREAM_BUFFER: usize = 128;
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
            .map(|spec| pb::Agent {
                id: spec.id.to_string(),
                kind: spec.kind.to_string(),
                command: spec.command().map(ToString::to_string).unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(pb::ListAgentsResponse { agents }))
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
            .await
            .map_err(|error| Status::failed_precondition(error.to_string()))?;

        Ok(Response::new(pb::CreateSessionResponse {
            session: Some((&session.meta()).into()),
        }))
    }

    async fn list_sessions(
        &self,
        _request: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        let sessions = self.registry.list().await.iter().map(Into::into).collect();

        Ok(Response::new(pb::ListSessionsResponse { sessions }))
    }

    async fn get_session(
        &self,
        request: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::GetSessionResponse>, Status> {
        let session = self.session(&request.into_inner().session).await?;

        Ok(Response::new(pb::GetSessionResponse {
            session: Some((&session.meta()).into()),
        }))
    }

    async fn delete_session(
        &self,
        request: Request<pb::DeleteSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        let id = request.into_inner().session;
        let id = SessionId::from_str(&id)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;

        self.registry
            .remove(id)
            .await
            .map_err(|error| Status::internal(error.to_string()))?;

        Ok(Response::new(pb::DeleteSessionResponse {}))
    }

    type AttachStream = ReceiverStream<Result<pb::Event, Status>>;

    async fn attach(
        &self,
        request: Request<pb::AttachRequest>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let attachment = session.attach(AttachOptions {
            replay: usize::try_from(request.replay).unwrap_or(DEFAULT_REPLAY),
            cursor: request.cursor.map(Cursor::from_raw),
        });

        let (sender, receiver) = mpsc::channel(STREAM_BUFFER);

        // The spawned task ends when the client disconnects, because sending on
        // a closed channel fails once tonic drops the response stream.
        tokio::spawn(async move {
            let mut stream = std::pin::pin!(attachment.into_stream());

            while let Some(item) = stream.next().await {
                let event = match item {
                    Ok(event) => Ok(pb::Event::from(&event)),
                    Err(AttachError::Lagged(skipped)) => {
                        let _ = sender
                            .send(Err(Status::data_loss(format!(
                                "client fell {skipped} events behind; re-attach from its last cursor"
                            ))))
                            .await;
                        return;
                    }
                    Err(AttachError::Closed) => return,
                };

                if sender.send(event).await.is_err() {
                    return;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    async fn prompt(
        &self,
        request: Request<pb::PromptRequest>,
    ) -> Result<Response<pb::PromptResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let content = request
            .content
            .into_iter()
            .map(ContentBlock::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| Status::invalid_argument(error.to_string()))?;

        let turn = session
            .prompt(content)
            .await
            .map_err(|error| Status::internal(error.to_string()))?;

        Ok(Response::new(pb::PromptResponse {
            turn: turn.to_string(),
        }))
    }

    async fn cancel(
        &self,
        request: Request<pb::CancelRequest>,
    ) -> Result<Response<pb::CancelResponse>, Status> {
        let session = self.session(&request.into_inner().session).await?;
        session
            .cancel()
            .await
            .map_err(|error| Status::internal(error.to_string()))?;

        Ok(Response::new(pb::CancelResponse {}))
    }

    async fn set_title(
        &self,
        request: Request<pb::SetTitleRequest>,
    ) -> Result<Response<pb::SetTitleResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        session
            .set_title(&request.title)
            .await
            .map_err(|error| Status::internal(error.to_string()))?;

        Ok(Response::new(pb::SetTitleResponse {}))
    }

    async fn load_history(
        &self,
        request: Request<pb::LoadHistoryRequest>,
    ) -> Result<Response<pb::LoadHistoryResponse>, Status> {
        let request = request.into_inner();
        let session = self.session(&request.session).await?;
        let limit = usize::try_from(request.limit)
            .unwrap_or(DEFAULT_PAGE)
            .max(1);
        let page = session.page(request.after.map_or(Cursor::START, Cursor::from_raw), limit);

        Ok(Response::new(pb::LoadHistoryResponse {
            events: page.events.iter().map(Into::into).collect(),
            next: page.next.map(Cursor::raw),
        }))
    }
}
