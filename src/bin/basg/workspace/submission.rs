//! Applies submission state transitions to the editor; the outbox owns correlation and drafts.

use super::*;
use bas::model::{PromptRejection, PromptRejectionKind, PromptSubmission, PromptSubmissionOutcome};
use std::sync::Arc;

impl Workspace {
    pub(super) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        let Some(session) = self.selected.id else {
            return;
        };
        let draft = self.composer.draft(cx);
        if self.working()
            || self
                .sessions
                .outboxes
                .get(&session)
                .is_some_and(outbox::SessionOutbox::pending)
        {
            self.enqueue_draft(session, draft.text, window, cx);
            return;
        }
        self.selected.error = None;
        if self.composer_fills_thread() {
            self.composer.expanded = false;
            self.sync_composer_mode(cx);
        }
        self.composer.commands.dismissed = true;
        self.selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        let title = self.current().filter(|meta| meta.title.is_none()).map(|_| {
            draft
                .text
                .lines()
                .find(|line| !line.trim().is_empty())
                .or_else(|| draft.attachments.first().map(|item| item.name.as_str()))
                .unwrap_or_default()
                .chars()
                .take(48)
                .collect()
        });
        let send = self
            .sessions
            .outboxes
            .entry(session)
            .or_default()
            .start(draft, outbox::Origin::Composer);
        let submission = crate::submission::prepare(
            session,
            send.turn,
            &send.draft.text,
            &send.draft.attachments,
            &self.prompt_capabilities(),
        );
        self.command(
            Command::Prompt {
                session,
                submission,
                title,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn resolve_submission(
        &mut self,
        session: SessionId,
        retry: bool,
        cx: &mut Context<Self>,
    ) {
        if retry && (self.archive_blocked(session) || !self.sessions.healthy(session)) {
            return;
        }
        let Some(submission) = self
            .sessions
            .outboxes
            .get_mut(&session)
            .and_then(|outbox| outbox.resolve())
        else {
            return;
        };
        self.command(
            Command::ResolvePrompt {
                session,
                submission,
                retry,
            },
            cx,
        );
        cx.notify();
    }

    /// Returns whether an owned submission settled and queue dispatch may be reconsidered.
    pub(super) fn apply_submission(
        &mut self,
        session: SessionId,
        submission: Arc<PromptSubmission>,
        result: Result<PromptSubmissionOutcome, Failure>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // Title preflight did not attempt the prompt; RPC failures after submission remain unknown.
        let result = match result {
            Err(Failure::Preflight(error)) => {
                Ok(PromptSubmissionOutcome::Rejected(PromptRejection {
                    kind: PromptRejectionKind::Unavailable,
                    message: error.presentation(),
                }))
            }
            other => other.map_err(|error| error.presentation()),
        };
        let settlement = self
            .sessions
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
                    let edited = if self.selected.id == Some(session) {
                        !self.composer.matches(&draft, cx)
                    } else {
                        self.sessions.drafts.get(&session).is_some_and(|current| {
                            !draft.matches(&current.text, &current.attachments)
                        })
                    };
                    if edited {
                        // The original remains available without replacing a newer composition.
                        self.sessions
                            .outboxes
                            .get_mut(&session)
                            .expect("settled outbox")
                            .retain_rejected_composition(draft);
                    } else {
                        self.sessions.drafts.entry(session).or_insert(draft);
                    }
                }
                self.session_error(
                    session,
                    crate::i18n::tr!(
                        "message-send-error",
                        session = session.to_string(),
                        error = message
                    ),
                );
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
        if let Some(draft) = self.sessions.drafts.get_mut(&session)
            && draft.matches(&submitted.text, &submitted.attachments)
        {
            *draft = Draft::default();
        }
        if self.selected.id == Some(session) && self.composer.matches(submitted, cx) {
            self.composer.replace(Draft::default(), window, cx);
        }
    }
}
