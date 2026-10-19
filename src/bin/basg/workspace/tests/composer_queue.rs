use super::*;

#[gpui_kit::test]
fn pending_panel_edits_messages_and_withdrawing_returns_them_to_the_editor(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for text in ["A", "B", "C"] {
                view.enqueue_draft(meta.id, text.into(), window, cx);
            }
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("composition", window, cx));
        });
        window.render_frame(cx);
        let panel = window.find("pending-messages-panel").bounds();
        assert!(panel.bottom() <= window.find("conversation-status").bounds().top());
        window.click("edit-pending-message-1", cx);
        assert_eq!(fixture.view.read(cx).composer.input.read(cx).value(), "B");
        window.render_frame(cx);
        window.click("withdraw-pending-message-0", cx);
        window.render_frame(cx);
        window.click("withdraw-pending-message-0", cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.composer.input.read(cx).value(), "B");
        assert_eq!(
            view.sessions.outboxes[&meta.id]
                .front()
                .expect("remaining queued message")
                .text,
            "C"
        );
        assert_eq!(view.sessions.outboxes[&meta.id].len(), 1);
    })
    .expect("pending panel window");
}

#[gpui_kit::test]
fn withdrawing_a_queued_message_loads_it_into_the_editor_and_escape_restores_the_composition(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for text in ["A", "B"] {
                view.enqueue_draft(meta.id, text.into(), window, cx);
            }
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("composition", window, cx));
        });
        window.render_frame(cx);
        window.click("withdraw-pending-message-0", cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.composer.input.read(cx).value(), "A");
        assert_eq!(
            view.sessions.outboxes[&meta.id]
                .front()
                .expect("remaining queued message")
                .text,
            "B"
        );
        window.press("escape", cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.composer.input.read(cx).value(), "composition");
        assert_eq!(view.sessions.outboxes[&meta.id].len(), 1);
    })
    .expect("withdraw to editor");
}

#[gpui_kit::test]
fn sending_the_last_withdrawn_message_restores_the_composition_and_clears_the_edit(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions
                .outboxes
                .entry(meta.id)
                .or_default()
                .enqueue(Draft {
                    text: "A".into(),
                    attachments: Vec::new(),
                });
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("composition", window, cx));
        });
        window.render_frame(cx);
        window.click("withdraw-pending-message-0", cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.composer.input.read(cx).value(), "A");
        assert_eq!(view.sessions.outboxes[&meta.id].len(), 0);
        window.click("send-message", cx);
    })
    .expect("send a withdrawn message");
    let Command::Prompt { submission, .. } = fixture.commands.try_recv().expect("prompt") else {
        panic!("expected prompt");
    };
    assert_eq!(submission::text(&submission), "A");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.composer.input.read(cx).value(), "composition");
        assert!(!view.sessions.outboxes[&meta.id].editing_active());
        window.render_frame(cx);
    })
    .expect("restored composition");
}

#[gpui_kit::test]
fn immediate_queue_send_waits_for_cancel_and_completion_and_keeps_other_drafts_paused(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let previous = bas::model::TurnId::new();
    let mut meta = session();
    meta.status = SessionStatus::Working;
    meta.active_turn = Some(previous);
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.enqueue_draft(meta.id, "A".into(), window, cx);
            view.enqueue_draft(meta.id, "B".into(), window, cx);
            view.sessions.outboxes.get_mut(&meta.id).expect("queue").pause();
        });
        window.render_frame(cx);
        window.click("send-pending-message-1", cx);
        assert!(matches!(fixture.commands.try_recv().expect("interrupt command"), Command::Cancel(id) if id == meta.id));
        assert!(fixture.commands.try_recv().is_err());
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Cancelled { session: meta.id, result: Ok(()) }, window, cx);
        });
        assert!(fixture.commands.try_recv().is_err(), "cancel acknowledgement is not turn completion");
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Event {
                epoch: view.selected.epoch,
                event: Event::new(meta.id, Cursor::from_raw(1), EventKind::TurnFinished {
                    turn: previous,
                    result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::Cancelled)),
                }),
            }, window, cx);
        });
        let command = fixture.commands.try_recv().expect("send selected draft");
        let Command::Prompt { submission, .. } = &command else {
            panic!("expected selected prompt");
        };
        assert_eq!(submission::text(submission), "B");
        let submission = submission::payload(command);
        let accepted = bas::model::TurnReceipt { turn: submission.turn, cursor: Cursor::from_raw(2) };
        fixture.view.update(cx, |view, cx| {
            view.update(submission::accepted_at(meta.id, &submission, 2), window, cx);
            view.update(Update::Event {
                epoch: view.selected.epoch,
                event: Event::new(meta.id, Cursor::from_raw(3), EventKind::TurnFinished {
                    turn: accepted.turn,
                    result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
                }),
            }, window, cx);
        });
        assert!(fixture.commands.try_recv().is_err());
        assert_eq!(fixture.view.read(cx).sessions.outboxes[&meta.id].front().expect("paused A").text, "A");
    }).expect("immediate send window");
}

#[gpui_kit::test]
fn finishing_before_cancel_acknowledgement_keeps_the_immediate_draft_until_success(
    cx: &mut TestAppContext,
) {
    for succeeded in [true, false] {
        let mut fixture = fixture(cx);
        let previous = bas::model::TurnId::new();
        let mut meta = session();
        meta.status = SessionStatus::Working;
        meta.active_turn = Some(previous);
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.enqueue_draft(meta.id, "原队列消息".into(), window, cx);
                view.sessions.outboxes.get_mut(&meta.id).expect("queue").pause();
                view.composer.input.update(cx, |input, cx| input.set_value("立即发送的新消息", window, cx));
            });
            window.render_frame(cx);
            window.click("send-immediately", cx);
            assert!(matches!(fixture.commands.try_recv().expect("cancel current turn"), Command::Cancel(id) if id == meta.id));
            fixture.view.update(cx, |view, cx| {
                view.update(Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(meta.id, Cursor::from_raw(1), EventKind::TurnFinished {
                        turn: previous,
                        result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
                    }),
                }, window, cx);
            });
            assert!(fixture.commands.try_recv().is_err(), "a finished turn cannot bypass cancel acknowledgement");
            fixture.view.update(cx, |view, cx| {
                view.update(Update::Cancelled {
                    session: meta.id,
                    result: if succeeded { Ok(()) } else { Err("cancel RPC failed".into()) },
                }, window, cx);
            });
            if succeeded {
                let Command::Prompt { submission, .. } = fixture.commands.try_recv().expect("acknowledged immediate send") else {
                    panic!("expected immediate prompt");
                };
                assert_eq!(submission::text(&submission), "立即发送的新消息");
                assert_eq!(fixture.view.read(cx).sessions.outboxes[&meta.id].front().expect("paused original message").text, "原队列消息");
            } else {
                assert!(fixture.commands.try_recv().is_err(), "failed cancellation keeps the draft queued");
                let view = fixture.view.read(cx);
                assert!(view.sessions.outboxes[&meta.id].paused());
                assert_eq!(view.sessions.outboxes[&meta.id].front().expect("failed immediate draft").text, "立即发送的新消息");
                assert_eq!(view.sessions.outboxes[&meta.id].len(), 2);
                assert!(view.sessions.errors.get(&meta.id).expect("cancellation error").contains("cancel RPC failed"));
            }
        }).expect("early completion window");
    }
}
