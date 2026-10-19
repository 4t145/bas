use super::*;

#[gpui_kit::test]
fn tracked_outcome_does_not_skip_metadata_and_waits_for_idle_catalog(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut background = session();
    background.revision = Cursor::from_raw(3);
    background.status = SessionStatus::Working;
    let foreground = session();
    attach(&fixture, foreground.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let outbox = view.sessions.outboxes.entry(background.id).or_default();
            let send = outbox.start(Draft { text: "accepted".into(), attachments: Vec::new() }, outbox::Origin::Composer);
            let accepted = bas::model::TurnReceipt { turn: send.turn, cursor: Cursor::from_raw(3) };
            background.active_turn = Some(send.turn);
            view.sessions.entries.push(background.clone());
            outbox.settle(std::sync::Arc::new(bas::model::PromptSubmission {
                turn: send.turn,
                request: bas::acp::PromptRequest::new(background.id.to_string(), vec!["accepted".into()]),
            }), Ok(PromptSubmissionOutcome::Accepted(accepted)));
            outbox.enqueue(Draft { text: "unsent".into(), attachments: Vec::new() });
            view.update(Update::TrackedTurn {
                event: Event::new(background.id, Cursor::from_raw(5), EventKind::TurnFinished {
                    turn: accepted.turn,
                    result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
                }),
            }, window, cx);
            assert!(fixture.commands.try_recv().is_err(), "Working metadata still gates dispatch");
            assert_eq!(view.sessions.entries.iter().find(|meta| meta.id == background.id).expect("background").revision, accepted.cursor);
            assert_eq!(view.selected.timeline.conversation.as_ref().expect("foreground").session.id, foreground.id);
            background.revision = Cursor::from_raw(5);
            background.status = SessionStatus::Idle;
            background.active_turn = None;
            background.title = Some("intermediate title update".into());
            view.update(Update::Catalog(Ok(connection::Catalog {
                agents: view.backend.agents.clone(),
                sessions: vec![background.clone().into(), foreground.into()],
            })), window, cx);
            assert_eq!(view.sessions.entries.iter().find(|meta| meta.id == background.id).expect("complete metadata").title, background.title);
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session, submission, .. }) if session == background.id && submission::text(&submission) == "unsent"));
    }).expect("tracked completion and catalog");
}
