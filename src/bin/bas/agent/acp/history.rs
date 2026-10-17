//! Agent-owned history discovery and deletion, without creating a session.

use std::{collections::HashSet, path::PathBuf};

use agent_client_protocol::schema::v1;

use super::management::with_agent;
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
