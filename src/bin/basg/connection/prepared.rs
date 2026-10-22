//! UI ingress accounting. Payload text is measured on the connection task, never
//! while parking a timeline. Presentation is charged separately by the timeline.

use super::*;

fn weight(value: &impl serde::Serialize) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

#[derive(Clone, Debug)]
pub(crate) struct Prepared<T> {
    pub(crate) value: T,
    pub(crate) bytes: usize,
}

impl<T: serde::Serialize> From<T> for Prepared<T> {
    fn from(value: T) -> Self {
        Self {
            bytes: weight(&value),
            value,
        }
    }
}

impl<T> std::ops::Deref for Prepared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HistoryWeight {
    events: Vec<usize>,
    event_bytes: usize,
    context_bytes: usize,
    session_bytes: usize,
}

impl HistoryWeight {
    pub(crate) fn bytes(&self) -> usize {
        self.event_bytes
            .saturating_add(self.context_bytes)
            .saturating_add(self.session_bytes)
            // Window structure, vector slots, and accounting itself.
            .saturating_add(std::mem::size_of::<HistoryWindow>())
    }

    pub(crate) fn append(&mut self, bytes: usize) {
        let bytes =
            bytes.saturating_add(std::mem::size_of::<Event>() + std::mem::size_of::<usize>());
        self.events.push(bytes);
        self.event_bytes = self.event_bytes.saturating_add(bytes);
    }

    pub(crate) fn retain_prefix(&mut self, old: &Self, prefix: usize) {
        self.event_bytes = old.events[..prefix]
            .iter()
            .fold(self.event_bytes, |total, bytes| {
                total.saturating_add(*bytes)
            });
        self.events
            .splice(0..0, old.events[..prefix].iter().copied());
        self.context_bytes = old.context_bytes;
    }

    pub(crate) fn prepend(&mut self, page: Self) {
        self.event_bytes = self.event_bytes.saturating_add(page.event_bytes);
        self.events.splice(0..0, page.events);
        self.context_bytes = page.context_bytes;
    }

    pub(crate) fn merge_meta(&mut self, bytes: usize) {
        // A newer snapshot replaces metadata; repeated polls must not grow the
        // budget when the retained snapshot has not grown.
        self.session_bytes = self.session_bytes.max(bytes);
    }

    pub(crate) fn suffix_bytes(&self, start: usize) -> usize {
        self.events[start..].iter().fold(
            self.session_bytes.saturating_add(self.context_bytes),
            |total, bytes| total.saturating_add(*bytes),
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedHistory {
    pub(crate) value: HistoryWindow,
    pub(crate) weight: HistoryWeight,
}

impl From<HistoryWindow> for PreparedHistory {
    fn from(value: HistoryWindow) -> Self {
        let mut accounting = HistoryWeight {
            session_bytes: weight(&value.session),
            context_bytes: weight(&value.context).saturating_add(
                value
                    .context
                    .len()
                    .saturating_mul(std::mem::size_of::<Event>()),
            ),
            ..Default::default()
        };
        for event in &value.events {
            accounting.append(weight(event));
        }
        Self {
            value,
            weight: accounting,
        }
    }
}

impl std::ops::Deref for PreparedHistory {
    type Target = HistoryWindow;

    fn deref(&self) -> &HistoryWindow {
        &self.value
    }
}
