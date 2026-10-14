use super::*;
use bas::{
    acp,
    model::{PromptRejection, PromptRejectionKind, PromptSubmission, TurnReceipt},
};
use std::sync::Arc;

pub(super) fn payload(command: Command) -> Arc<PromptSubmission> {
    let Command::Prompt { submission, .. } = command else {
        panic!("expected first submission");
    };
    submission
}

pub(super) fn text(submission: &PromptSubmission) -> &str {
    let [acp::ContentBlock::Text(text)] = submission.request.prompt.as_slice() else {
        panic!("expected a single text-only prompt");
    };
    &text.text
}

fn unknown(session: SessionId, submission: &Arc<PromptSubmission>) -> Update {
    Update::Submission {
        session,
        submission: submission.clone(),
        result: Err(connection::Failure::Rpc {
            code: tonic::Code::Unavailable,
            message: "response lost".into(),
            details: Vec::new(),
            diagnostic: "response lost".into(),
        }),
    }
}

fn accepted(session: SessionId, submission: &Arc<PromptSubmission>) -> Update {
    accepted_at(session, submission, 1)
}

pub(super) fn accepted_at(
    session: SessionId,
    submission: &Arc<PromptSubmission>,
    cursor: u64,
) -> Update {
    Update::Submission {
        session,
        submission: submission.clone(),
        result: Ok(PromptSubmissionOutcome::Accepted(TurnReceipt {
            turn: submission.turn,
            cursor: Cursor::from_raw(cursor),
        })),
    }
}

pub(super) fn rejected(
    session: SessionId,
    submission: &Arc<PromptSubmission>,
    message: &str,
) -> Update {
    Update::Submission {
        session,
        submission: submission.clone(),
        result: Ok(PromptSubmissionOutcome::Rejected(PromptRejection {
            kind: PromptRejectionKind::Unavailable,
            message: message.into(),
        })),
    }
}

#[gpui_kit::test]
fn prior_turn_replies_cannot_settle_new_composition_or_advance_queue(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let file = tempfile::NamedTempFile::new().expect("file");
    std::fs::write(file.path(), "attachment").expect("contents");
    let attachment = Attachment::load(file.path()).expect("attachment");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("prior", window, cx));
            view.send_prompt(window, cx);
        });
        let prior = payload(fixture.commands.try_recv().expect("prior prompt"));
        fixture.view.update(cx, |view, cx| {
            view.update(accepted(meta.id, &prior), window, cx);
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(2),
                        EventKind::TurnFinished {
                            turn: prior.turn,
                            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
                        },
                    ),
                },
                window,
                cx,
            );
            view.add_attachments(meta.id, vec![attachment.clone()], window, cx);
            view.send_prompt(window, cx);
            view.sessions
                .outboxes
                .get_mut(&meta.id)
                .expect("outbox")
                .enqueue(Draft {
                    text: "queued".into(),
                    attachments: Vec::new(),
                });
        });
        let current = payload(fixture.commands.try_recv().expect("current prompt"));
        fixture.view.update(cx, |view, cx| {
            let text = view.composer.input.read(cx).value().to_string();
            for reply in [
                rejected(meta.id, &prior, "late rejection"),
                accepted(meta.id, &prior),
            ] {
                view.update(reply, window, cx);
                assert!(view.sending(meta.id));
                assert_eq!(view.sessions.outboxes[&meta.id].turn(), Some(current.turn));
                assert_eq!(view.composer.input.read(cx).value().as_ref(), text);
                assert_eq!(view.composer.attachments[0].id, attachment.id);
                assert_eq!(
                    view.sessions.outboxes[&meta.id]
                        .front()
                        .expect("queued")
                        .text,
                    "queued"
                );
                assert!(!view.sessions.outboxes[&meta.id].paused());
            }
        });
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("window");
}

#[gpui_kit::test]
fn invalid_acceptance_retains_unknown_immutable_payload(cx: &mut TestAppContext) {
    for wrong_turn in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        let file = tempfile::NamedTempFile::new().expect("file");
        std::fs::write(file.path(), "immutable contents").expect("contents");
        let attachment = Attachment::load(file.path()).expect("attachment");
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.add_attachments(meta.id, vec![attachment.clone()], window, cx);
                view.send_prompt(window, cx);
            });
            let original = payload(fixture.commands.try_recv().expect("prompt"));
            fixture.view.update(cx, |view, cx| {
                view.update(
                    Update::Submission {
                        session: meta.id,
                        submission: original.clone(),
                        result: Ok(PromptSubmissionOutcome::Accepted(TurnReceipt {
                            turn: if wrong_turn {
                                bas::model::TurnId::new()
                            } else {
                                original.turn
                            },
                            cursor: Cursor::from_raw(if wrong_turn { 1 } else { 0 }),
                        })),
                    },
                    window,
                    cx,
                );
                assert!(view.sending(meta.id));
                assert!(!view.composition_locked(meta.id));
                assert_eq!(view.composer.attachments[0].id, attachment.id);
                assert!(view.sessions.outboxes[&meta.id].unknown().is_some());
            });
            window.render_frame(cx);
            window.click("retry-submission", cx);
            let Command::ResolvePrompt {
                submission,
                retry: true,
                ..
            } = fixture.commands.try_recv().expect("explicit retry")
            else {
                panic!("retry")
            };
            assert!(Arc::ptr_eq(&original, &submission));
            assert_eq!(original.request, submission.request);
            assert!(fixture.commands.try_recv().is_err());
        })
        .expect("window");
    }
}

#[gpui_kit::test]
fn preflight_failure_restores_composition_or_queue_without_unknown(cx: &mut TestAppContext) {
    for queued in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                if queued {
                    view.enqueue_draft(meta.id, "retained".into(), window, cx);
                } else {
                    view.composer
                        .input
                        .update(cx, |input, cx| input.set_value("retained", window, cx));
                    view.send_prompt(window, cx);
                }
            });
            let submission = payload(fixture.commands.try_recv().expect("submission"));
            fixture.view.update(cx, |view, cx| {
                view.update(
                    Update::Submission {
                        session: meta.id,
                        submission,
                        result: Err(connection::Failure::Preflight(Box::new(
                            "preflight rejected".into(),
                        ))),
                    },
                    window,
                    cx,
                );
                assert!(!view.sending(meta.id));
                assert!(view.sessions.outboxes[&meta.id].unknown().is_none());
                assert!(
                    view.composer_error(cx)
                        .expect("failure feedback")
                        .contains("preflight rejected")
                );
                if queued {
                    assert_eq!(
                        view.sessions.outboxes[&meta.id]
                            .front()
                            .expect("requeued")
                            .text,
                        "retained"
                    );
                    assert!(view.sessions.outboxes[&meta.id].paused());
                } else {
                    assert_eq!(view.composer.input.read(cx).value(), "retained");
                    assert_eq!(view.sessions.drafts[&meta.id].text, "retained");
                }
            });
            assert!(fixture.commands.try_recv().is_err(), "no implicit retry");
        })
        .expect("window");
    }
}

#[gpui_kit::test]
fn closed_local_command_channel_restores_composer_or_requeues_without_unknown(
    cx: &mut TestAppContext,
) {
    for queued in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        fixture.commands.close();
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                if queued {
                    view.enqueue_draft(meta.id, "retained".into(), window, cx);
                } else {
                    view.composer
                        .input
                        .update(cx, |input, cx| input.set_value("retained", window, cx));
                    view.send_prompt(window, cx);
                }
                assert!(!view.sending(meta.id));
                assert!(view.sessions.outboxes[&meta.id].unknown().is_none());
                assert!(!view.backend.connected);
                assert_eq!(
                    view.backend
                        .error
                        .as_ref()
                        .map(connection::Failure::presentation)
                        .as_deref(),
                    Some(crate::i18n::tr!("connection-reopen"))
                );
                assert_eq!(
                    view.composer_error(cx),
                    view.backend
                        .error
                        .as_ref()
                        .map(connection::Failure::presentation)
                );
                if queued {
                    assert_eq!(
                        view.sessions.outboxes[&meta.id]
                            .front()
                            .expect("requeued")
                            .text,
                        "retained"
                    );
                    assert!(view.sessions.outboxes[&meta.id].paused());
                } else {
                    assert_eq!(view.composer.input.read(cx).value(), "retained");
                    assert_eq!(view.sessions.drafts[&meta.id].text, "retained");
                }
            });
        })
        .expect("window");
    }
}

#[gpui_kit::test]
fn unknown_actions_preserve_payload_attachments_and_new_edits_until_background_acceptance(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let file = tempfile::NamedTempFile::new().expect("file");
    std::fs::write(file.path(), "original attachment").expect("file data");
    let attachment = Attachment::load(file.path()).expect("attachment");
    let edited = format!("edited via keyboard {}", attachment.mention);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.add_attachments(first.id, vec![attachment.clone()], window, cx);
            view.send_prompt(window, cx);
        });
        let original = payload(fixture.commands.try_recv().expect("first prompt"));
        fixture.view.update(cx, |view, cx| {
            view.update(unknown(first.id, &original), window, cx);
            assert!(view.sending(first.id));
            assert!(!view.composition_locked(first.id));
            assert_eq!(view.composer.attachments[0].id, attachment.id);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("new draft", window, cx));
            view.save_draft(cx);
            view.send_prompt(window, cx);
            view.send_immediately(window, cx);
            view.flush_queue(cx);
        });
        assert!(
            fixture.commands.try_recv().is_err(),
            "unknown cannot implicitly resend"
        );
        window.render_frame(cx);
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.focus(window, cx));
        });
        window.press("ctrl-a", cx);
        window.input(&edited, cx);
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.composer.input.read(cx).value().as_ref(), edited);
            view.save_draft(cx);
        });
        window.click("query-submission", cx);
        let Command::ResolvePrompt {
            submission,
            retry: false,
            ..
        } = fixture.commands.try_recv().expect("explicit query")
        else {
            panic!("query")
        };
        assert!(Arc::ptr_eq(&original, &submission));
        fixture.view.update(cx, |view, cx| {
            view.update(unknown(first.id, &original), window, cx)
        });
        window.render_frame(cx);
        window.click("retry-submission", cx);
        let Command::ResolvePrompt {
            submission,
            retry: true,
            ..
        } = fixture.commands.try_recv().expect("explicit retry")
        else {
            panic!("retry")
        };
        assert!(
            Arc::ptr_eq(&original, &submission),
            "retry must not rebuild attachments or text"
        );
        fixture.view.update(cx, |view, cx| {
            view.update(unknown(first.id, &original), window, cx);
            view.sessions.entries.push(second.clone());
            view.select(second.id, window, cx);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("other session", window, cx));
            view.update(accepted(first.id, &original), window, cx);
            assert_eq!(
                view.composer.input.read(cx).value().as_ref(),
                "other session"
            );
            assert_eq!(view.sessions.drafts[&first.id].text, edited);
            assert_eq!(
                view.sessions.drafts[&first.id].attachments[0].id,
                attachment.id
            );
            assert!(!view.sending(first.id));
            view.select(first.id, window, cx);
            assert_eq!(view.composer.input.read(cx).value().as_ref(), edited);
            assert_eq!(view.composer.attachments[0].id, attachment.id);
        });
    })
    .expect("window");
}

#[gpui_kit::test]
fn acceptance_clears_only_unchanged_compositions_in_current_and_background_sessions(
    cx: &mut TestAppContext,
) {
    for background in [false, true] {
        for changed in [false, true] {
            let mut fixture = fixture(cx);
            let first = session();
            let second = session();
            attach(&fixture, first.clone(), cx);
            while fixture.commands.try_recv().is_ok() {}
            let file = tempfile::NamedTempFile::new().expect("file");
            std::fs::write(file.path(), "shared attachment").expect("contents");
            let attachment = Attachment::load(file.path()).expect("attachment");
            cx.update_window(fixture.window.into(), |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.add_attachments(first.id, vec![attachment.clone()], window, cx);
                    view.send_prompt(window, cx);
                });
                let original = payload(fixture.commands.try_recv().expect("prompt"));
                fixture.view.update(cx, |view, cx| {
                    view.update(unknown(first.id, &original), window, cx);
                    let edited = format!("new question about {}", attachment.mention);
                    if changed {
                        view.composer
                            .input
                            .update(cx, |input, cx| input.set_value(&edited, window, cx));
                    }
                    view.save_draft(cx);
                    if background {
                        view.sessions.entries.push(second.clone());
                        view.select(second.id, window, cx);
                    }
                    view.update(accepted(first.id, &original), window, cx);
                    let saved = &view.sessions.drafts[&first.id];
                    if changed {
                        assert_eq!(saved.text, edited);
                        assert_eq!(saved.attachments.len(), 1);
                        assert_eq!(saved.attachments[0].id, attachment.id);
                        assert!(Arc::ptr_eq(
                            &saved.attachments[0].content,
                            &attachment.content
                        ));
                    } else {
                        assert!(saved.text.is_empty());
                        assert!(saved.attachments.is_empty());
                    }
                    if background {
                        view.select(first.id, window, cx);
                    }
                    if changed {
                        assert_eq!(view.composer.input.read(cx).value().as_ref(), edited);
                        assert_eq!(view.composer.attachments[0].id, attachment.id);
                    } else {
                        assert!(view.composer.input.read(cx).value().is_empty());
                        assert!(view.composer.attachments.is_empty());
                    }
                });
            })
            .expect("window");
        }
    }
}

#[gpui_kit::test]
fn unknown_queue_waits_for_matching_acceptance_and_real_termination(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.enqueue_draft(meta.id, "A".into(), window, cx);
            view.enqueue_draft(meta.id, "B".into(), window, cx);
        });
        let original = payload(fixture.commands.try_recv().expect("queued A"));
        fixture.view.update(cx, |view, cx| {
            view.update(unknown(meta.id, &original), window, cx);
            view.sessions
                .outboxes
                .get_mut(&meta.id)
                .expect("outbox")
                .resume();
            view.flush_queue(cx);
            let unrelated = Arc::new(PromptSubmission::new(original.request.clone()));
            view.update(accepted(meta.id, &unrelated), window, cx);
            assert!(view.sending(meta.id));
            view.update(accepted(meta.id, &original), window, cx);
            assert!(!view.sending(meta.id));
        });
        assert!(
            fixture.commands.try_recv().is_err(),
            "acceptance is not completion"
        );
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::TrackedTurn {
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(2),
                        EventKind::TurnFinished {
                            turn: original.turn,
                            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
                        },
                    ),
                },
                window,
                cx,
            );
        });
        let next = payload(
            fixture
                .commands
                .try_recv()
                .expect("queued B after real completion"),
        );
        assert_ne!(next.turn, original.turn);
        fixture.view.update(cx, |view, cx| {
            view.update(unknown(meta.id, &original), window, cx);
            view.update(accepted(meta.id, &original), window, cx);
            assert_eq!(view.sessions.outboxes[&meta.id].turn(), Some(next.turn));
            assert!(view.sessions.outboxes[&meta.id].unknown().is_none());
        });
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("window");
}

#[gpui_kit::test]
fn explicit_rejection_restores_queue_pauses_and_next_attempt_gets_new_identity(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.enqueue_draft(meta.id, "A".into(), window, cx)
        });
        let original = payload(fixture.commands.try_recv().expect("A"));
        fixture.view.update(cx, |view, cx| {
            view.update(unknown(meta.id, &original), window, cx);
            view.update(
                Update::Submission {
                    session: meta.id,
                    submission: original.clone(),
                    result: Ok(PromptSubmissionOutcome::Rejected(PromptRejection {
                        kind: PromptRejectionKind::Busy,
                        message: "not accepted".into(),
                    })),
                },
                window,
                cx,
            );
            assert!(!view.sending(meta.id));
            assert!(view.sessions.outboxes[&meta.id].paused());
            assert_eq!(
                view.sessions.outboxes[&meta.id]
                    .front()
                    .expect("restored")
                    .text,
                "A"
            );
        });
        assert!(fixture.commands.try_recv().is_err());
        window.render_frame(cx);
        window.click("resume-pending-messages", cx);
        let next = payload(fixture.commands.try_recv().expect("explicit resume"));
        assert_ne!(original.turn, next.turn);
        assert_eq!(original.request, next.request);
    })
    .expect("window");
}

#[gpui_kit::test]
fn rejection_keeps_new_composition_and_retains_original_before_other_queued_drafts(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("original", window, cx));
            view.send_prompt(window, cx);
        });
        let original = payload(fixture.commands.try_recv().expect("original"));
        fixture.view.update(cx, |view, cx| {
            view.update(unknown(meta.id, &original), window, cx);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("new draft", window, cx));
            view.save_draft(cx);
            view.sessions
                .outboxes
                .get_mut(&meta.id)
                .expect("outbox")
                .enqueue(Draft {
                    text: "queued later".into(),
                    attachments: Vec::new(),
                });
            view.update(
                Update::Submission {
                    session: meta.id,
                    submission: original,
                    result: Ok(PromptSubmissionOutcome::Rejected(PromptRejection {
                        kind: PromptRejectionKind::Unavailable,
                        message: "rejected".into(),
                    })),
                },
                window,
                cx,
            );
            assert_eq!(view.composer.input.read(cx).value().as_ref(), "new draft");
            assert_eq!(view.sessions.drafts[&meta.id].text, "new draft");
            assert!(view.sessions.outboxes[&meta.id].paused());
            assert_eq!(
                view.sessions.outboxes[&meta.id]
                    .iter()
                    .map(|draft| draft.text.as_str())
                    .collect::<Vec<_>>(),
                ["original", "queued later"]
            );
        });
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("window");
}
