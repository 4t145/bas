use std::{collections::VecDeque, sync::Arc};

use basr::model::{
    Cursor, Event, EventKind, PromptSubmission, PromptSubmissionOutcome, SessionMeta,
    SessionStatus, TurnId, TurnReceipt,
};

use crate::attachment::Draft;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    Composer,
    Queue,
}

enum Dispatch {
    Ready,
    Submitting {
        turn: TurnId,
        attempt: Attempt,
        draft: Draft,
        origin: Origin,
        finished: Option<Completion>,
    },
    Awaiting(TurnReceipt),
}

enum Attempt {
    Sending,
    Unknown(Arc<PromptSubmission>),
    Resolving(Arc<PromptSubmission>),
}

struct Completion {
    cursor: Cursor,
    interrupted: bool,
}

pub(super) struct Send {
    pub turn: TurnId,
    pub draft: Draft,
}

pub(super) enum Settlement {
    Accepted {
        origin: Origin,
        draft: Draft,
    },
    Rejected {
        origin: Origin,
        draft: Draft,
        message: String,
    },
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
    #[cfg(test)]
    pub fn turn(&self) -> Option<TurnId> {
        match &self.dispatch {
            Dispatch::Submitting { turn, .. } => Some(*turn),
            _ => None,
        }
    }

    pub fn unknown(&self) -> Option<&Arc<PromptSubmission>> {
        match &self.dispatch {
            Dispatch::Submitting {
                attempt: Attempt::Unknown(submission) | Attempt::Resolving(submission),
                ..
            } => Some(submission),
            _ => None,
        }
    }

    pub fn resolving(&self) -> bool {
        matches!(
            self.dispatch,
            Dispatch::Submitting {
                attempt: Attempt::Resolving(_),
                ..
            }
        )
    }

    pub fn resolve(&mut self) -> Option<Arc<PromptSubmission>> {
        if self.resolving() {
            return None;
        }
        let submission = self.unknown()?.clone();
        if let Dispatch::Submitting { attempt, .. } = &mut self.dispatch {
            *attempt = Attempt::Resolving(submission.clone());
        }
        Some(submission)
    }

    pub fn resolution_not_sent(&mut self, turn: TurnId) {
        if let Dispatch::Submitting {
            turn: pending,
            attempt,
            ..
        } = &mut self.dispatch
            && *pending == turn
            && let Attempt::Resolving(submission) = attempt
        {
            *attempt = Attempt::Unknown(submission.clone());
        }
    }

    pub fn composition_locked(&self) -> bool {
        matches!(
            self.dispatch,
            Dispatch::Submitting {
                attempt: Attempt::Sending,
                ..
            }
        )
    }

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

    pub fn retain_rejected_composition(&mut self, draft: Draft) {
        self.queue.push_front(draft);
        self.pause();
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

    pub fn start(&mut self, draft: Draft, origin: Origin) -> Send {
        assert!(matches!(self.dispatch, Dispatch::Ready));
        let turn = TurnId::new();
        self.dispatch = Dispatch::Submitting {
            turn,
            attempt: Attempt::Sending,
            draft: draft.clone(),
            origin,
            finished: None,
        };
        Send { turn, draft }
    }

    pub fn settle(
        &mut self,
        submission: Arc<PromptSubmission>,
        result: Result<PromptSubmissionOutcome, String>,
    ) -> Option<Settlement> {
        let Dispatch::Submitting { turn, attempt, .. } = &mut self.dispatch else {
            return None;
        };
        if *turn != submission.turn {
            return None;
        }
        match attempt {
            Attempt::Unknown(original) | Attempt::Resolving(original)
                if !Arc::ptr_eq(original, &submission) && **original != *submission =>
            {
                return None;
            }
            _ => {}
        }
        let receipt = match result {
            Ok(PromptSubmissionOutcome::Accepted(receipt))
                if receipt.turn == *turn && receipt.cursor.raw() > 0 =>
            {
                receipt
            }
            Ok(PromptSubmissionOutcome::Rejected(rejection)) => {
                return self.reject(rejection.message);
            }
            _ => {
                let original = match attempt {
                    Attempt::Sending => submission,
                    Attempt::Unknown(original) | Attempt::Resolving(original) => original.clone(),
                };
                *attempt = Attempt::Unknown(original);
                return None;
            }
        };
        let Dispatch::Submitting {
            draft,
            origin,
            finished,
            ..
        } = std::mem::replace(&mut self.dispatch, Dispatch::Ready)
        else {
            return None;
        };
        if let Some(completion) = finished.filter(|event| event.cursor > receipt.cursor) {
            if completion.interrupted {
                self.pause();
            }
        } else {
            self.dispatch = Dispatch::Awaiting(receipt);
        }
        Some(Settlement::Accepted { origin, draft })
    }

    pub fn not_sent(&mut self, turn: TurnId, message: String) -> Option<Settlement> {
        if !matches!(self.dispatch, Dispatch::Submitting {
            turn: pending, attempt: Attempt::Sending, ..
        } if pending == turn)
        {
            return None;
        }
        self.reject(message)
    }

    fn reject(&mut self, message: String) -> Option<Settlement> {
        let Dispatch::Submitting { draft, origin, .. } =
            std::mem::replace(&mut self.dispatch, Dispatch::Ready)
        else {
            return None;
        };
        self.pause();
        if origin == Origin::Queue {
            self.queue.push_front(draft.clone());
        }
        Some(Settlement::Rejected {
            origin,
            draft,
            message,
        })
    }

    pub fn observe(&mut self, event: &Event) {
        if let EventKind::TurnFinished { turn, .. } | EventKind::TurnInterrupted { turn, .. } =
            event.kind
        {
            let interrupted = matches!(event.kind, EventKind::TurnInterrupted { .. });
            match &mut self.dispatch {
                Dispatch::Submitting {
                    turn: submitted,
                    finished,
                    ..
                } if *submitted == turn
                    && finished
                        .as_ref()
                        .is_none_or(|previous| event.cursor > previous.cursor) =>
                {
                    *finished = Some(Completion {
                        cursor: event.cursor,
                        interrupted,
                    });
                }
                Dispatch::Awaiting(receipt)
                    if receipt.turn == turn && event.cursor > receipt.cursor =>
                {
                    self.dispatch = Dispatch::Ready;
                    if interrupted {
                        self.pause();
                    }
                }
                _ => {}
            }
        }
    }

    pub fn advance(&mut self, meta: &SessionMeta, allowed: bool) -> Option<Send> {
        if meta.archived {
            self.pause();
            return None;
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

    pub fn tracking_failed(&mut self, failed: TurnReceipt) -> bool {
        if matches!(&self.dispatch, Dispatch::Awaiting(receipt) if *receipt == failed) {
            self.dispatch = Dispatch::Ready;
            self.pause();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests;
