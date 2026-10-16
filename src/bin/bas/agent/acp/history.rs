//! Agent-owned history discovery and deletion, without creating a session.

use std::{collections::HashSet, future::Future, path::PathBuf};

use agent_client_protocol::{Agent, Client, ConnectionTo, schema::v1};
use tokio::sync::oneshot;

use crate::agent::{AgentSpec, Error};

const MAX_DISCOVERED_SESSIONS: usize = 10_000;

pub struct Listing {
    pub initialization: v1::InitializeResponse,
    pub sessions: v1::ListSessionsResponse,
}

/// # Errors
/// Rejects unsupported discovery, invalid/repeating pagination, oversized lists
/// and transport or ACP errors. All pages use one connection; cursors are ephemeral.
pub async fn list(spec: &AgentSpec, cwd: Option<PathBuf>) -> Result<Listing, Error> {
    let agent = spec.id.clone();
    with_agent(spec, async move |cx, initialization| {
        if initialization
            .agent_capabilities
            .session_capabilities
            .list
            .is_none()
        {
            return Err(Error::Unsupported {
                agent,
                operation: "session/list",
            });
        }
        let mut sessions = Vec::new();
        let mut cursor = None;
        let mut seen = HashSet::new();
        loop {
            let page = cx
                .send_request(
                    v1::ListSessionsRequest::new()
                        .cwd(cwd.clone())
                        .cursor(cursor),
                )
                .block_task()
                .await?;
            sessions.extend(page.sessions);
            if sessions.len() > MAX_DISCOVERED_SESSIONS {
                return Err(Error::backend(
                    agent,
                    "Agent history exceeds the discovery limit; filter by workspace",
                ));
            }
            let Some(next) = page.next_cursor else { break };
            if !seen.insert(next.clone()) {
                return Err(Error::backend(
                    agent,
                    "Agent history repeated a pagination cursor",
                ));
            }
            cursor = Some(next);
        }
        Ok(Listing {
            initialization,
            sessions: v1::ListSessionsResponse::new(sessions),
        })
    })
    .await
}

/// # Errors
/// Returns Unsupported unless deletion was advertised, or the native ACP error.
pub async fn delete(spec: &AgentSpec, id: String) -> Result<(), Error> {
    let agent = spec.id.clone();
    with_agent(spec, async move |cx, initialization| {
        if initialization
            .agent_capabilities
            .session_capabilities
            .delete
            .is_none()
        {
            return Err(Error::Unsupported {
                agent,
                operation: "session/delete",
            });
        }
        cx.send_request(v1::DeleteSessionRequest::new(id))
            .block_task()
            .await?;
        Ok(())
    })
    .await
}

async fn with_agent<T, F, Fut>(spec: &AgentSpec, operation: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce(ConnectionTo<Agent>, v1::InitializeResponse) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, Error>> + Send + 'static,
{
    let transport = super::convert::agent(spec)?;
    let (send, receive) = oneshot::channel();
    // No filesystem, terminal or interactive capabilities are advertised here.
    // History browsing must not create a session or authorize tool execution.
    let connection =
        Client
            .builder()
            .name("bas-history")
            .connect_with(transport, async move |cx| {
                let result = async {
                    let initialization = v1::InitializeRequest::new(
                        agent_client_protocol::schema::ProtocolVersion::V1,
                    );
                    let initialized = cx.send_request(initialization).block_task().await?;
                    operation(cx, initialized).await
                }
                .await;
                let _ = send.send(result);
                std::future::pending::<()>().await;
                Ok(())
            });
    tokio::pin!(connection);
    tokio::select! {
        biased;
        result = receive => result.map_err(|_| Error::backend(spec.id.clone(), "Agent history connection closed"))?,
        result = &mut connection => Err(Error::backend(spec.id.clone(), format!("Agent history connection failed: {result:?}"))),
    }
}
