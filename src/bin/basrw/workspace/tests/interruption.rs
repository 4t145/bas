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
            let outbox = view.outboxes.entry(background.id).or_default();
            let send = outbox.start(Draft { text: "accepted".into(), attachments: Vec::new() }, outbox::Origin::Composer);
            let accepted = basr::model::TurnReceipt { turn: send.turn, cursor: Cursor::from_raw(3) };
            background.active_turn = Some(send.turn);
            view.sessions.push(background.clone());
            outbox.settle(std::sync::Arc::new(basr::model::PromptSubmission {
                turn: send.turn,
                request: basr::acp::PromptRequest::new(background.id.to_string(), vec!["accepted".into()]),
            }), Ok(PromptSubmissionOutcome::Accepted(accepted)));
            outbox.enqueue(Draft { text: "unsent".into(), attachments: Vec::new() });
            view.update(Update::TrackedTurn {
                event: Event::new(background.id, Cursor::from_raw(5), EventKind::TurnFinished {
                    turn: accepted.turn,
                    result: Ok(basr::acp::PromptResponse::new(basr::acp::StopReason::EndTurn)),
                }),
            }, window, cx);
            assert!(fixture.commands.try_recv().is_err(), "Working metadata still gates dispatch");
            assert_eq!(view.sessions.iter().find(|meta| meta.id == background.id).expect("background").revision, accepted.cursor);
            assert_eq!(view.conversation.as_ref().expect("foreground").session.id, foreground.id);
            background.revision = Cursor::from_raw(5);
            background.status = SessionStatus::Idle;
            background.active_turn = None;
            background.title = Some("intermediate title update".into());
            view.update(Update::Catalog(Ok(connection::Catalog {
                agents: view.agents.clone(),
                sessions: vec![background.clone().into(), foreground.into()],
            })), window, cx);
            assert_eq!(view.sessions.iter().find(|meta| meta.id == background.id).expect("complete metadata").title, background.title);
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session, text, .. }) if session == background.id && text == "unsent"));
    }).expect("tracked completion and catalog");
}

#[gpui_kit::test]
fn tracking_failure_is_visible_and_queue_requires_explicit_resume(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let outbox = view.outboxes.entry(session.id).or_default();
            let send = outbox.start(Draft { text: "accepted".into(), attachments: Vec::new() }, outbox::Origin::Composer);
            let accepted = basr::model::TurnReceipt { turn: send.turn, cursor: Cursor::from_raw(3) };
            outbox.settle(std::sync::Arc::new(basr::model::PromptSubmission {
                turn: send.turn,
                request: basr::acp::PromptRequest::new(session.id.to_string(), vec!["accepted".into()]),
            }), Ok(PromptSubmissionOutcome::Accepted(accepted)));
            outbox.enqueue(Draft { text: "unsent".into(), attachments: Vec::new() });
            view.update(Update::TrackingFailed {
                session: session.id,
                receipt: accepted,
                message: "cursor evicted".into(),
            }, window, cx);
            assert!(view.error.as_ref().is_some_and(|error| error.contains("cursor evicted")));
            assert!(view.outboxes.get(&session.id).expect("outbox").paused());
            assert!(fixture.commands.try_recv().is_err());
            view.outboxes.get_mut(&session.id).expect("outbox").resume();
            view.flush_queue(cx);
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "unsent"));
    }).expect("explicit retry");
}

#[gpui_kit::test]
fn switching_to_another_directory_never_replays_a_queue_after_restart(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut first = session();
    first.cwd = Some(PathBuf::from("/first"));
    let mut second = session();
    second.cwd = Some(PathBuf::from("/second"));
    attach(&fixture, first.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let accepted = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer
                    .update(cx, |input, cx| input.set_value("accepted", window, cx));
                view.send_prompt(window, cx);
                let submission =
                    submission::payload(fixture.commands.try_recv().expect("accepted prompt"));
                view.outboxes
                    .get_mut(&first.id)
                    .expect("outbox")
                    .enqueue(Draft {
                        text: "unsent".into(),
                        attachments: Vec::new(),
                    });
                view.update(
                    submission::accepted_at(first.id, &submission, 3),
                    window,
                    cx,
                );
                basr::model::TurnReceipt {
                    turn: submission.turn,
                    cursor: Cursor::from_raw(3),
                }
            })
        })
        .expect("submit accepted prompt");
    attach(&fixture, second.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let mut restarted = first.clone();
            restarted.revision = Cursor::from_raw(4);
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.agents.clone(),
                    sessions: vec![restarted.into(), second.into()],
                })),
                window,
                cx,
            );
            assert!(fixture.commands.try_recv().is_err());
            view.update(
                Update::TrackedTurn {
                    event: Event::new(
                        first.id,
                        Cursor::from_raw(4),
                        EventKind::TurnInterrupted {
                            turn: accepted.turn,
                            reason: basr::model::InterruptionReason::DaemonRestarted,
                        },
                    ),
                },
                window,
                cx,
            );
            let outbox = view.outboxes.get(&first.id).expect("background outbox");
            assert!(outbox.paused());
            assert_eq!(outbox.front().expect("retained draft").text, "unsent");
        });
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("background restart catalog");
}
