use super::*;
use prost::Message as _;

const MAX_LOAD_TIME: Duration = Duration::from_secs(10 * 60);
const MAX_HISTORY_BYTES: usize = 128 * 1024 * 1024;

/// Read a retained history with a deadline for each RPC, not for the entire
/// conversation. Large histories may legitimately take many successful pages.
///
/// # Errors
/// Reports stalled RPCs, invalid payloads and non-advancing pagination.
pub(super) async fn snapshot(
    client: &mut Client,
    session: SessionId,
    timeout: Duration,
    progress: impl FnMut(usize),
) -> Result<Snapshot, Failure> {
    tokio::time::timeout(
        MAX_LOAD_TIME,
        load_pages(client, session, timeout, progress),
    )
    .await
    .map_err(|_| Failure::Condition(Condition::HistoryTimeLimit))?
}

async fn load_pages(
    client: &mut Client,
    session: SessionId,
    timeout: Duration,
    mut progress: impl FnMut(usize),
) -> Result<Snapshot, Failure> {
    let started = std::time::Instant::now();
    tracing::debug!(%session, "Loading conversation history");
    let mut events = Vec::new();
    let mut after = None;
    let mut pages = 0usize;
    let mut bytes = 0usize;
    loop {
        let page = tokio::time::timeout(
            timeout,
            client.load_history(pb::LoadHistoryRequest {
                session: session.to_string(),
                after,
                limit: HISTORY_PAGE_SIZE,
            }),
        )
        .await
        .map_err(|_| Failure::Timeout)?
        .map_err(Failure::from)?
        .into_inner();
        account_bytes(&mut bytes, page.encoded_len())?;
        if page
            .next
            .is_some_and(|next| next <= after.unwrap_or(0) || page.events.is_empty())
        {
            return Err(Failure::Condition(Condition::RepeatedHistoryCursor));
        }
        for event in page.events {
            let event: Event = event
                .try_into()
                .map_err(|error: bas::service::Error| Failure::InvalidPayload(error.to_string()))?;
            if event.session != session {
                return Err(Failure::InvalidPayload(
                    "history returned another session".into(),
                ));
            }
            events.push(event);
        }
        pages += 1;
        progress(events.len());
        tracing::debug!(%session, pages, loaded = events.len(), next = ?page.next, "Loaded history page");
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    let meta = tokio::time::timeout(
        timeout,
        client.get_session(pb::GetSessionRequest {
            session: session.to_string(),
        }),
    )
    .await
    .map_err(|_| Failure::Timeout)?
    .map_err(Failure::from)?
    .into_inner()
    .session
    .ok_or(Failure::Condition(Condition::MissingSessionInfo))?
    .try_into()
    .map_err(|error: bas::service::Error| Failure::InvalidPayload(error.to_string()))?;
    tracing::info!(%session, pages, loaded = events.len(), elapsed_ms = started.elapsed().as_millis(), "Conversation history loaded");
    Ok(Snapshot {
        session: meta,
        events,
    })
}

fn account_bytes(total: &mut usize, page: usize) -> Result<(), Failure> {
    *total = total.saturating_add(page);
    if *total > MAX_HISTORY_BYTES {
        return Err(Failure::Condition(Condition::HistorySizeLimit));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
