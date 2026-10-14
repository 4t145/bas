//! Byte-bounded protobuf history pages. Conversion stays lazy: only the returned
//! events and at most one event that did not fit are converted.

use bas::service::pb;
use prost::Message;
use tonic::Status;

pub(super) struct PageEncoder {
    response: pb::LoadHistoryResponse,
    event_bytes: usize,
    budget: usize,
}

impl PageEncoder {
    pub(super) fn new(budget: usize) -> Self {
        Self {
            response: pb::LoadHistoryResponse::default(),
            event_bytes: 0,
            budget,
        }
    }

    /// Returns false when the caller must stop visiting history. The rejected
    /// event remains immediately after `next` and is never silently skipped.
    ///
    /// # Errors
    /// Returns `ResourceExhausted` if the first event and its envelope exceed the page budget.
    pub(super) fn push(&mut self, event: pb::Event, has_more: bool) -> Result<bool, Status> {
        // Use prost for both the repeated-message envelope and optional cursor:
        // this includes tags, length prefixes, and Some(0)'s presence.
        let next = has_more.then_some(event.cursor);
        let single = pb::LoadHistoryResponse {
            events: vec![event],
            next,
        };
        let next_bytes = pb::LoadHistoryResponse {
            events: Vec::new(),
            next,
        }
        .encoded_len();
        let single_bytes = single.encoded_len();
        if self.event_bytes + single_bytes > self.budget {
            if self.response.events.is_empty() {
                return Err(Status::resource_exhausted(format!(
                    "history event {} needs {single_bytes} protobuf bytes; page budget is {}",
                    single.events[0].cursor, self.budget,
                )));
            }
            // The previous iteration reserved its actual continuation cursor.
            return Ok(false);
        }
        self.event_bytes += single_bytes - next_bytes;
        self.response.events.extend(single.events);
        self.response.next = next;
        Ok(true)
    }

    pub(super) fn finish(self) -> pb::LoadHistoryResponse {
        self.response
    }
}

#[cfg(test)]
mod tests;
