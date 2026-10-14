use std::{collections::VecDeque, num::NonZeroUsize};

use bas::model::{Cursor, Event};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub events: Vec<Event>,
    pub next: Option<Cursor>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HistoryError {
    #[error(
        "history after cursor {requested} was evicted through {evicted_through}; load the current window without a cursor"
    )]
    Expired {
        requested: Cursor,
        evicted_through: Cursor,
    },
    #[error("cursor {requested} is ahead of the session's latest cursor {latest}")]
    Future { requested: Cursor, latest: Cursor },
    #[error("history page size must be greater than zero")]
    EmptyPage,
}

/// Retains native ACP updates and lifecycle events with explicit eviction cursors.
#[derive(Debug)]
pub(super) struct Ring {
    capacity: NonZeroUsize,
    events: VecDeque<Event>,
    evicted_through: Cursor,
}

impl Ring {
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity,
            events: VecDeque::new(),
            evicted_through: Cursor::START,
        }
    }

    pub fn push(&mut self, event: Event) {
        if self.events.len() == self.capacity.get() {
            self.evicted_through = self
                .events
                .pop_front()
                .expect("a full ring is nonempty")
                .cursor;
        }
        self.events.push_back(event);
    }

    /// # Errors
    /// Returns an error if the requested position has been evicted or limit is zero.
    pub fn page(&self, after: Option<Cursor>, limit: usize) -> Result<Page, HistoryError> {
        let mut page = Page::default();
        self.visit(after, limit, |event, more| {
            page.events.push(event.clone());
            page.next = more.then_some(event.cursor);
            true
        })?;
        Ok(page)
    }

    /// Visits borrowed events; `more` includes events outside the requested limit.
    ///
    /// # Errors
    /// Returns an error if the requested position has been evicted or limit is zero.
    pub fn visit(
        &self,
        after: Option<Cursor>,
        limit: usize,
        mut visitor: impl FnMut(&Event, bool) -> bool,
    ) -> Result<(), HistoryError> {
        if limit == 0 {
            return Err(HistoryError::EmptyPage);
        }
        if let Some(requested) = after
            && requested < self.evicted_through
        {
            return Err(HistoryError::Expired {
                requested,
                evicted_through: self.evicted_through,
            });
        }
        let after = after.unwrap_or(Cursor::START);
        let mut remaining = self
            .events
            .iter()
            .filter(|event| event.cursor > after)
            .peekable();
        for _ in 0..limit {
            let Some(event) = remaining.next() else { break };
            if !visitor(event, remaining.peek().is_some()) {
                break;
            }
        }
        Ok(())
    }

    pub fn tail(&self, count: usize) -> Vec<Event> {
        self.events
            .iter()
            .skip(self.events.len().saturating_sub(count))
            .cloned()
            .collect()
    }
}
