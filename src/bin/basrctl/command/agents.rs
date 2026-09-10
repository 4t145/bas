use anyhow::{Context, Result};
use basr::service::{Client, pb};

use crate::render;

/// Lists the daemon's configured agent backends.
///
/// # Errors
///
/// Returns an error when the daemon rejects the call.
pub(super) async fn list(client: &mut Client) -> Result<()> {
    let response = client
        .list_agents(pb::ListAgentsRequest {})
        .await
        .context("list agents failed")?
        .into_inner();

    render::agents(&response.agents);
    Ok(())
}
