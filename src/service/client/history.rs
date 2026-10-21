//! Validated raw-event history loaders shared by all frontends.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tonic::Status;

use super::Client;
use crate::{
    model::{Event, SessionId, SessionSnapshot},
    service::pb,
};

/// A safe recent-first range of the canonical event log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryWindow {
    pub session: SessionSnapshot,
    pub events: Vec<Event>,
    /// Earlier global state; replay in order before `events`. Replace this
    /// context with the older window's context when prepending, rather than
    /// retaining newer context ahead of older transcript events.
    pub context: Vec<Event>,
    pub before: Option<u64>,
    /// Attach cursor for the newest window, not an acknowledgement of older pages.
    pub cursor: u64,
}

impl Client {
    /// Loads a contiguous recent-first range, extending the target count to
    /// include message, tool and other replay dependencies.
    ///
    /// # Errors
    /// Returns `ResourceExhausted` for a safe range too large for one RPC,
    /// `DataLoss` for expired dependencies or an
    /// invalid response, and transport/deadline errors without retrying.
    pub async fn history_window(
        &mut self,
        session: SessionId,
        before: Option<u64>,
        limit: u32,
        timeout: Duration,
    ) -> Result<HistoryWindow, Status> {
        if limit == 0 {
            return Err(Status::invalid_argument(
                "history window limit must be positive",
            ));
        }
        let response = tokio::time::timeout(
            timeout,
            self.load_history_window(pb::LoadHistoryWindowRequest {
                session: session.to_string(),
                before,
                limit,
            }),
        )
        .await
        .map_err(|_| deadline())??
        .into_inner();
        decode_window(session, before, response)
    }
}

fn deadline() -> Status {
    Status::deadline_exceeded("history RPC timed out")
}

fn decode_session(id: SessionId, session: Option<pb::Session>) -> Result<SessionSnapshot, Status> {
    let session: SessionSnapshot = session
        .ok_or_else(|| Status::data_loss("history response is missing session metadata"))?
        .try_into()
        .map_err(invalid_payload)?;
    if session.meta.id != id {
        return Err(Status::data_loss(
            "history metadata belongs to another session",
        ));
    }
    Ok(session)
}

fn invalid_payload(error: crate::service::Error) -> Status {
    Status::data_loss(error.to_string())
}

fn decode_events(
    id: SessionId,
    events: Vec<pb::Event>,
    contiguous: bool,
) -> Result<Vec<Event>, Status> {
    let mut previous: Option<u64> = None;
    events
        .into_iter()
        .map(|event| {
            let event: Event = event.try_into().map_err(invalid_payload)?;
            let cursor = event.cursor.raw();
            if event.session != id || cursor == 0 || event.is_transient() {
                return Err(Status::data_loss(
                    "invalid history event identity or cursor",
                ));
            }
            if previous.is_some_and(|previous| {
                cursor <= previous || (contiguous && previous.checked_add(1) != Some(cursor))
            }) {
                return Err(Status::data_loss("history events are not in cursor order"));
            }
            previous = Some(cursor);
            Ok(event)
        })
        .collect()
}

fn decode_window(
    id: SessionId,
    requested_before: Option<u64>,
    response: pb::LoadHistoryWindowResponse,
) -> Result<HistoryWindow, Status> {
    let session = decode_session(id, response.session)?;
    if response.cursor != session.meta.revision.raw() {
        return Err(Status::data_loss("history cursor disagrees with snapshot"));
    }
    let events = decode_events(id, response.events, true)?;
    let context = decode_events(id, response.context, false)?;
    let upper = requested_before.map_or(response.cursor, |before| before.saturating_sub(1));
    if requested_before.is_some_and(|before| before > response.cursor.saturating_add(1))
        || events
            .last()
            .is_some_and(|event| event.cursor.raw() != upper)
        || (events.is_empty() && upper != 0)
    {
        return Err(Status::data_loss(
            "history window does not end at its requested cursor",
        ));
    }
    if let Some(before) = response.before
        && (before <= 1
            || events.first().map(|event| event.cursor.raw()) != Some(before)
            || requested_before.is_some_and(|requested| before >= requested))
    {
        return Err(Status::data_loss(
            "history returned an invalid older cursor",
        ));
    }
    if context.last().is_some_and(|event| {
        event.cursor.raw() >= events.first().map_or(0, |event| event.cursor.raw())
    }) {
        return Err(Status::data_loss(
            "history context overlaps the event window",
        ));
    }
    Ok(HistoryWindow {
        session,
        events,
        context,
        before: response.before,
        cursor: response.cursor,
    })
}

#[cfg(test)]
mod tests;
