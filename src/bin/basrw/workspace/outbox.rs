use std::collections::VecDeque;

use basr::model::{Cursor, Event, EventKind, SessionMeta, SessionStatus, TurnId, TurnReceipt};

use crate::attachment::Draft;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    Composer,
    Queue,
}

enum Dispatch {
    Ready,
    Submitting {
        draft: Draft,
        origin: Origin,
        finished: Vec<(TurnId, Cursor)>,
    },
    Awaiting(TurnReceipt),
}

pub(super) enum Action {
    Send(Draft),
    Accepted(Origin),
    Rejected { origin: Origin, draft: Draft },
}

/// Owns drafts from enqueue until acceptance. Metadata and replay progress are
/// independent: only a receipt-correlated completion releases an accepted send.
pub(super) struct SessionOutbox {
    queue: VecDeque<Draft>,
    dispatch: Dispatch,
    paused: bool,
    editing: Option<Draft>,
}

impl Default for SessionOutbox {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            dispatch: Dispatch::Ready,
            paused: false,
            editing: None,
        }
    }
}

impl SessionOutbox {
    pub fn sending(&self) -> bool {
        matches!(self.dispatch, Dispatch::Submitting { .. })
    }

    pub fn pending(&self) -> bool {
        !self.queue.is_empty() || !matches!(self.dispatch, Dispatch::Ready)
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn front(&self) -> Option<&Draft> {
        self.queue.front()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Submitting an edit replaces the reserved head; the displaced composition
    /// returns to the composer, not to the send queue.
    pub fn enqueue(&mut self, draft: Draft) -> Option<Draft> {
        self.paused = false;
        if let Some(composition) = self.editing.take() {
            *self
                .queue
                .front_mut()
                .expect("editing reserves the queue head") = draft;
            Some(composition)
        } else {
            self.queue.push_back(draft);
            None
        }
    }

    pub fn begin_edit(&mut self, composition: Draft) -> Option<Draft> {
        if self.editing.is_some() {
            return None;
        }
        let draft = self.queue.front()?.clone();
        self.editing = Some(composition);
        self.paused = true;
        Some(draft)
    }

    /// Escape discards only the unsubmitted edit; the original queued draft and
    /// the composition displaced by editing both remain owned.
    pub fn cancel_edit(&mut self) -> Option<Draft> {
        self.editing.take()
    }

    pub fn start(&mut self, draft: Draft, origin: Origin) -> Action {
        assert!(matches!(self.dispatch, Dispatch::Ready));
        self.dispatch = Dispatch::Submitting {
            draft: draft.clone(),
            origin,
            finished: Vec::new(),
        };
        Action::Send(draft)
    }

    pub fn receipt(&mut self, receipt: Option<TurnReceipt>) -> Option<Action> {
        if !self.sending() {
            return None;
        }
        let Dispatch::Submitting {
            draft,
            origin,
            finished,
        } = std::mem::replace(&mut self.dispatch, Dispatch::Ready)
        else {
            return None;
        };
        match receipt {
            Some(receipt) => {
                if !finished
                    .iter()
                    .any(|(turn, cursor)| *turn == receipt.turn && *cursor > receipt.cursor)
                {
                    self.dispatch = Dispatch::Awaiting(receipt);
                }
                Some(Action::Accepted(origin))
            }
            None => {
                if origin == Origin::Queue {
                    self.queue.push_front(draft.clone());
                    self.paused = true;
                }
                Some(Action::Rejected { origin, draft })
            }
        }
    }

    pub fn observe(&mut self, event: &Event) {
        if let EventKind::TurnFinished { turn, .. } = event.kind {
            match &mut self.dispatch {
                Dispatch::Submitting { finished, .. } => finished.push((turn, event.cursor)),
                Dispatch::Awaiting(receipt)
                    if receipt.turn == turn && event.cursor > receipt.cursor =>
                {
                    self.dispatch = Dispatch::Ready;
                }
                _ => {}
            }
        }
    }

    pub fn advance(&mut self, meta: &SessionMeta, allowed: bool) -> Option<Action> {
        if meta.archived {
            self.pause();
            return None;
        }
        if let Dispatch::Awaiting(receipt) = &self.dispatch
            && receipt.cursor != Cursor::START
            && meta.revision >= receipt.cursor
            && meta.status == SessionStatus::Idle
            && meta.active_turn.is_none()
        {
            self.dispatch = Dispatch::Ready;
        }
        if !allowed
            || self.paused
            || self.editing.is_some()
            || !matches!(self.dispatch, Dispatch::Ready)
            || meta.status != SessionStatus::Idle
            || meta.active_turn.is_some()
        {
            return None;
        }
        let draft = self.queue.pop_front()?;
        Some(self.start(draft, Origin::Queue))
    }
}

#[cfg(test)]
mod tests;
