use anyhow::{Context, Result};
use basr::service::{Client, pb};

use crate::render;

/// Prints a page of history and the cursor to continue from.
///
/// # Errors
///
/// Returns an error when the daemon rejects the call.
pub(super) async fn page(
    client: &mut Client,
    session: &str,
    after: Option<u64>,
    limit: u32,
) -> Result<()> {
    let response = client
        .load_history(pb::LoadHistoryRequest {
            session: session.to_owned(),
            after,
            limit,
        })
        .await
        .context("load history failed")?
        .into_inner();

    let mut timeline = render::Timeline::default();
    for event in &response.events {
        timeline.event(event)?;
    }
    timeline.finish()?;
    render::next_cursor(response.next);

    Ok(())
}
