use std::collections::VecDeque;

use crate::model::{Cursor, Event, EventKind};

/// Number of replayable events a session keeps when its configuration does not
/// say otherwise.
pub const DEFAULT_CAPACITY: usize = 4096;

/// A page of replayable events, as returned to a client that loads history.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub events: Vec<Event>,
    /// Cursor to pass to the next call, absent when this page is the last one.
    pub next: Option<Cursor>,
}

impl Page {
    /// Returns whether more events are available after this page.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.next.is_some()
    }
}

/// Bounded, in-memory history of one session.
///
/// Persistence is the agent backend's responsibility (see the design document);
/// the daemon keeps a replay window so clients can attach, rewind, and page
/// through recent events without waking the backend.
#[derive(Debug)]
pub struct Ring {
    capacity: usize,
    events: VecDeque<Event>,
}

impl Ring {
    /// Creates a ring that keeps at most `capacity` events.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            events: VecDeque::new(),
        }
    }

    /// Appends a replayable event, evicting the oldest when full.
    pub fn push(&mut self, event: Event) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Returns events strictly after `after`, up to `limit` of them.
    ///
    /// [`Page::next`] is the cursor to resume from, and is `None` when the page
    /// reached the newest event.
    #[must_use]
    pub fn page(&self, after: Cursor, limit: usize) -> Page {
        let limit = limit.max(1);
        let events: Vec<Event> = self
            .events
            .iter()
            .filter(|event| event.cursor > after)
            .take(limit)
            .cloned()
            .collect();

        let next = events
            .last()
            .filter(|last| self.events.iter().any(|event| event.cursor > last.cursor))
            .map(|last| last.cursor);

        Page { events, next }
    }

    /// Returns the newest `count` replayable events, oldest first.
    #[must_use]
    pub fn tail(&self, count: usize) -> Vec<Event> {
        let skip = self.events.len().saturating_sub(count);
        self.events.iter().skip(skip).cloned().collect()
    }

    /// Returns the newest `count` committed messages, oldest first.
    #[must_use]
    pub fn messages(&self, count: usize) -> Vec<Event> {
        self.events
            .iter()
            .rev()
            .filter(|event| matches!(event.kind, EventKind::Message(_)))
            .take(count)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .cloned()
            .collect()
    }

    /// Number of events currently kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns whether no event is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Maximum number of events this ring keeps.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
