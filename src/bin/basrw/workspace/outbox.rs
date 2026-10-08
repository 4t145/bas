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

struct Editing {
    index: usize,
    composition: Draft,
}

pub(super) struct Withdrawal {
    pub composition: Option<Draft>,
}

/// Owns drafts from enqueue until acceptance. Metadata and replay progress are
/// independent: only a receipt-correlated completion releases an accepted send.
pub(super) struct SessionOutbox {
    queue: VecDeque<Draft>,
    dispatch: Dispatch,
    paused: bool,
    editing: Option<Editing>,
    priority: bool,
    interrupting: bool,
}

impl Default for SessionOutbox {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            dispatch: Dispatch::Ready,
            paused: false,
            editing: None,
            priority: false,
            interrupting: false,
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

    pub fn iter(&self) -> impl Iterator<Item = &Draft> {
        self.queue.iter()
    }

    pub fn editing(&self) -> Option<usize> {
        self.editing.as_ref().map(|editing| editing.index)
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
        self.priority = false;
        self.interrupting = false;
    }

    pub fn resume(&mut self) {
        if self.editing.is_none() {
            self.paused = false;
        }
    }

    /// Submitting an edit replaces its reserved slot; the displaced composition
    /// returns to the composer, not to the send queue.
    pub fn enqueue(&mut self, draft: Draft) -> Option<Draft> {
        self.paused = false;
        if let Some(editing) = self.editing.take() {
            *self
                .queue
                .get_mut(editing.index)
                .expect("editing reserves a queue slot") = draft;
            Some(editing.composition)
        } else {
            self.queue.push_back(draft);
            None
        }
    }

    pub fn enqueue_immediate(&mut self, draft: Draft, interrupt: bool) -> Option<Draft> {
        let (index, composition) = if let Some(editing) = self.editing.take() {
            *self
                .queue
                .get_mut(editing.index)
                .expect("editing reserves a queue slot") = draft;
            (editing.index, Some(editing.composition))
        } else {
            self.queue.push_back(draft);
            (self.queue.len() - 1, None)
        };
        assert!(self.prioritize(index, interrupt));
        composition
    }

    #[cfg(test)]
    pub fn begin_edit(&mut self, composition: Draft) -> Option<Draft> {
        self.begin_edit_at(0, composition)
    }

    pub fn begin_edit_at(&mut self, index: usize, composition: Draft) -> Option<Draft> {
        if self.editing.is_some() || self.sending() || self.interrupting {
            return None;
        }
        let draft = self.queue.get(index)?.clone();
        self.editing = Some(Editing { index, composition });
        self.paused = true;
        Some(draft)
    }

    /// Escape discards only the unsubmitted edit; the original queued draft and
    /// the composition displaced by editing both remain owned.
    pub fn cancel_edit(&mut self) -> Option<Draft> {
        self.editing.take().map(|editing| editing.composition)
    }

    pub fn withdraw(&mut self, index: usize) -> Option<Withdrawal> {
        if self.sending() || self.interrupting {
            return None;
        }
        self.queue.remove(index)?;
        let composition = match self.editing.as_mut() {
            Some(editing) if editing.index == index => self.cancel_edit(),
            Some(editing) if editing.index > index => {
                editing.index -= 1;
                None
            }
            _ => None,
        };
        if index == 0 {
            self.priority = false;
        }
        Some(Withdrawal { composition })
    }

    /// One explicit send may bypass a paused queue without resuming its other drafts.
    pub fn prioritize(&mut self, index: usize, interrupt: bool) -> bool {
        if self.sending() || self.editing.is_some() || self.interrupting {
            return false;
        }
        let Some(draft) = self.queue.remove(index) else {
            return false;
        };
        self.queue.push_front(draft);
        self.priority = true;
        self.interrupting = interrupt;
        true
    }

    pub fn complete_interrupt(&mut self, succeeded: bool) {
        if !self.interrupting {
            return;
        }
        self.interrupting = false;
        if !succeeded {
            self.pause();
        }
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
            || (self.paused && !self.priority)
            || self.interrupting
            || self.editing.is_some()
            || !matches!(self.dispatch, Dispatch::Ready)
            || meta.status != SessionStatus::Idle
            || meta.active_turn.is_some()
        {
            return None;
        }
        let draft = self.queue.pop_front()?;
        self.priority = false;
        Some(self.start(draft, Origin::Queue))
    }
}

#[cfg(test)]
mod tests;
