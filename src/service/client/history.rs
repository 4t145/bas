//! Validated raw-event history loaders shared by all frontends.

use std::time::{Duration, Instant};

use prost::Message as _;
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

/// Complete prompt navigation metadata, independent of loaded body windows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryOverview {
    pub session: SessionId,
    pub cursor: u64,
    pub entries: Vec<HistoryOverviewEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryOverviewEntry {
    pub cursor: u64,
    pub timestamp_millis: i64,
    pub preview: String,
}

/// A directly addressed logical page with dependency-complete older context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryLocation {
    pub window: HistoryWindow,
    /// Next logical page start; pass to `history_window_at` to move forwards.
    pub after: Option<u64>,
}

impl Client {
    /// Loads every prompt marker without loading transcript bodies.
    ///
    /// # Errors
    /// Returns transport/deadline errors, `ResourceExhausted` if the complete
    /// overview cannot fit, or `DataLoss` for incomplete or malformed history.
    pub async fn history_overview(
        &mut self,
        session: SessionId,
        timeout: Duration,
    ) -> Result<HistoryOverview, Status> {
        let response = tokio::time::timeout(
            timeout,
            self.load_history_overview(pb::LoadHistoryOverviewRequest {
                session: session.to_string(),
            }),
        )
        .await
        .map_err(|_| deadline())??
        .into_inner();
        decode_overview(session, response)
    }

    /// Loads the complete logical page containing `cursor` directly.
    /// Dependencies may expand it backwards, never through unrelated newer pages.
    ///
    /// # Errors
    /// Returns `InvalidArgument` for zero/future cursors, storage/transport errors,
    /// `ResourceExhausted` for an oversized page, or `DataLoss` for invalid replies.
    pub async fn history_window_at(
        &mut self,
        session: SessionId,
        cursor: u64,
        timeout: Duration,
    ) -> Result<HistoryLocation, Status> {
        if cursor == 0 {
            return Err(Status::invalid_argument(
                "history location cursor must be positive",
            ));
        }
        let response = tokio::time::timeout(
            timeout,
            self.load_history_window_at(pb::LoadHistoryWindowAtRequest {
                session: session.to_string(),
                cursor,
            }),
        )
        .await
        .map_err(|_| deadline())??
        .into_inner();
        decode_location(session, cursor, response)
    }

    /// Loads `limit` complete user turns, newest first in chronological order.
    /// Initialization is a separate segment. Replay dependencies can expand the
    /// range by whole turns; a long turn is never silently split.
    ///
    /// # Errors
    /// Returns `InvalidArgument` for zero limit or a non-boundary `before`,
    /// `ResourceExhausted` for a safe range too large for one RPC,
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
        let started = Instant::now();
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
        let rpc_ms = started.elapsed().as_millis();
        let protobuf_bytes = response.encoded_len();
        let decoding = Instant::now();
        let window = decode_window(session, before, response)?;
        tracing::debug!(
            %session, ?before, requested_turns = limit,
            events = window.events.len(), protobuf_bytes, rpc_ms,
            decode_ms = decoding.elapsed().as_millis(),
            "Loaded history window"
        );
        Ok(window)
    }
}

fn deadline() -> Status {
    Status::deadline_exceeded("history RPC timed out")
}

fn decode_overview(
    id: SessionId,
    response: pb::LoadHistoryOverviewResponse,
) -> Result<HistoryOverview, Status> {
    if response.session.parse::<SessionId>().ok() != Some(id) {
        return Err(Status::data_loss(
            "history overview belongs to another session",
        ));
    }
    let mut previous = 0;
    let mut entries = Vec::with_capacity(response.entries.len());
    for entry in response.entries {
        if entry.cursor <= previous
            || entry.cursor > response.cursor
            || entry.timestamp_millis < 0
            || entry.preview.chars().count() > crate::model::history::PROMPT_PREVIEW_CHARS
        {
            return Err(Status::data_loss("invalid history overview entry"));
        }
        previous = entry.cursor;
        entries.push(HistoryOverviewEntry {
            cursor: entry.cursor,
            timestamp_millis: entry.timestamp_millis,
            preview: entry.preview,
        });
    }
    Ok(HistoryOverview {
        session: id,
        cursor: response.cursor,
        entries,
    })
}

fn decode_location(
    id: SessionId,
    requested: u64,
    response: pb::LoadHistoryWindowAtResponse,
) -> Result<HistoryLocation, Status> {
    let raw = response
        .window
        .ok_or_else(|| Status::data_loss("missing history window"))?;
    if response
        .after
        .is_some_and(|after| after <= requested || after > raw.cursor)
    {
        return Err(Status::data_loss("invalid next history page cursor"));
    }
    let window = decode_window(id, response.after, raw)?;
    if requested == 0
        || window
            .events
            .first()
            .is_none_or(|event| event.cursor.raw() > requested)
        || window
            .events
            .last()
            .is_none_or(|event| event.cursor.raw() < requested)
        || window.before
            != window
                .events
                .first()
                .filter(|event| event.cursor.raw() > 1)
                .map(|event| event.cursor.raw())
    {
        return Err(Status::data_loss(
            "history window does not contain requested cursor",
        ));
    }
    Ok(HistoryLocation {
        window,
        after: response.after,
    })
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
