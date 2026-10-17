//! Short-lived, capability-checked operations without creating a conversation.

use crate::agent::{AgentSpec, Error};
use agent_client_protocol::{Agent, Client, ConnectionTo, schema::v1};
use std::future::Future;
use tokio::sync::oneshot;

/// # Errors
/// Rejects Agents without advertised logout support and preserves native failures.
pub async fn logout(spec: &AgentSpec) -> Result<(), Error> {
    let agent = spec.id.clone();
    with_agent(spec, async move |cx, initialized| {
        if initialized.agent_capabilities.auth.logout.is_none() {
            return Err(Error::Unsupported {
                agent,
                operation: "logout",
            });
        }
        cx.send_request(v1::LogoutRequest::new())
            .block_task()
            .await?;
        Ok(())
    })
    .await
}

/// # Errors
/// Returns transport, initialization, connection or operation failures.
pub(super) async fn with_agent<T, F, Fut>(spec: &AgentSpec, operation: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce(ConnectionTo<Agent>, v1::InitializeResponse) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, Error>> + Send + 'static,
{
    let transport = super::convert::agent(spec)?;
    let (send, receive) = oneshot::channel();
    // Management must not create a session or grant filesystem, terminal or
    // interactive execution capabilities merely to inspect history or log out.
    let connection =
        Client
            .builder()
            .name("bas-management")
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
        result = receive => result.map_err(|_| Error::backend(spec.id.clone(), "ACP management connection closed"))?,
        result = &mut connection => Err(Error::backend(spec.id.clone(), format!("ACP management connection failed: {result:?}"))),
    }
}
