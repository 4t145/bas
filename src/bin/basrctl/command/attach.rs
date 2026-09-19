use anyhow::{Context, Result};
use basr::service::{Client, pb};
use tokio_stream::StreamExt;

use crate::render;

/// Attaches to a session: prints the requested history, then streams events
/// until the process is interrupted.
///
/// # Errors
///
/// Returns an error when the daemon rejects the attachment or the stream fails.
pub(super) async fn run(
    client: &mut Client,
    session: &str,
    cursor: Option<u64>,
    replay: usize,
) -> Result<()> {
    let mut events = client
        .attach(pb::AttachRequest {
            session: session.to_owned(),
            cursor,
            replay: u32::try_from(replay).unwrap_or(u32::MAX),
        })
        .await
        .context("attach failed")?
        .into_inner();

    let mut timeline = render::Timeline::default();
    while let Some(event) = events.next().await {
        timeline.event(&event.context("the event stream failed")?)?;
    }

    timeline.finish()
}
