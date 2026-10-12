//! Applies submission state transitions to the editor; the outbox owns correlation and drafts.

use super::*;
use basr::model::{PromptSubmission, PromptSubmissionOutcome};
use std::sync::Arc;

impl Workspace {
    pub(super) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        let Some(session) = self.active else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        if self.working()
            || self
                .outboxes
                .get(&session)
                .is_some_and(outbox::SessionOutbox::pending)
        {
            self.enqueue_draft(session, text, window, cx);
            return;
        }
        self.error = None;
        if self.composer_fills_thread() {
            self.composer_expanded = false;
            self.sync_composer_mode(cx);
        }
        self.commands_dismissed = true;
        self.list.set_follow_mode(FollowMode::Tail);
        let title = self.current().filter(|meta| meta.title.is_none()).map(|_| {
            text.lines()
                .find(|line| !line.trim().is_empty())
                .or_else(|| self.attachments.first().map(|item| item.name.as_str()))
                .unwrap_or_default()
                .chars()
                .take(48)
                .collect()
        });
        let send = self.outboxes.entry(session).or_default().start(
            Draft {
                text,
                attachments: self.attachments.clone(),
            },
            outbox::Origin::Composer,
        );
        self.command(Command::Prompt {
            session,
            turn: send.turn,
            text: send.draft.text,
            attachments: send.draft.attachments,
            capabilities: self.prompt_capabilities(),
            title,
        });
        cx.notify();
    }

    pub(super) fn resolve_submission(
        &mut self,
        session: SessionId,
        retry: bool,
        cx: &mut Context<Self>,
    ) {
        if retry && (self.archive_blocked(session) || !self.runtime_healthy(session)) {
            return;
        }
        let Some(submission) = self
            .outboxes
            .get_mut(&session)
            .and_then(|outbox| outbox.resolve())
        else {
            return;
        };
        let turn = submission.turn;
        if !self.connection.send(Command::ResolvePrompt {
            session,
            submission,
            retry,
        }) {
            self.outboxes
                .get_mut(&session)
                .expect("resolving outbox")
                .resolution_not_sent(turn);
        }
        cx.notify();
    }

    /// Returns whether an owned submission settled and queue dispatch may be reconsidered.
    pub(super) fn apply_submission(
        &mut self,
        session: SessionId,
        submission: Arc<PromptSubmission>,
        result: Result<PromptSubmissionOutcome, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let settlement = self
            .outboxes
            .get_mut(&session)
            .and_then(|outbox| outbox.settle(submission, result));
        let Some(settlement) = settlement else {
            cx.notify();
            return false;
        };
        match settlement {
            outbox::Settlement::Accepted { origin, draft } => {
                self.record_user_prompt(session);
                if origin == outbox::Origin::Composer {
                    self.clear_submitted_composition(session, &draft, window, cx);
                }
            }
            outbox::Settlement::Rejected {
                origin,
                draft,
                message,
            } => {
                if origin == outbox::Origin::Composer {
                    let edited = if self.active == Some(session) {
                        !draft.matches(self.composer.read(cx).value().as_ref(), &self.attachments)
                    } else {
                        self.drafts.get(&session).is_some_and(|current| {
                            !draft.matches(&current.text, &current.attachments)
                        })
                    };
                    if edited {
                        // The original remains available without replacing a newer composition.
                        self.outboxes
                            .get_mut(&session)
                            .expect("settled outbox")
                            .retain_rejected_composition(draft);
                    } else {
                        self.drafts.entry(session).or_insert(draft);
                    }
                }
                self.error = Some(crate::i18n::tr!(
                    "message-send-error",
                    session = session.to_string(),
                    error = message
                ));
            }
        }
        true
    }

    fn clear_submitted_composition(
        &mut self,
        session: SessionId,
        submitted: &Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.drafts.get_mut(&session)
            && draft.matches(&submitted.text, &submitted.attachments)
        {
            *draft = Draft::default();
        }
        if self.active == Some(session)
            && submitted.matches(self.composer.read(cx).value().as_ref(), &self.attachments)
        {
            self.attachments.clear();
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }
}
