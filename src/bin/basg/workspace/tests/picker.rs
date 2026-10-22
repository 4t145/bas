use super::*;

fn finish_initialization(
    fixture: &Fixture,
    session: SessionMeta,
    events: Vec<Event>,
    window: &mut Window,
    cx: &mut App,
) {
    fixture.view.update(cx, |view, cx| {
        view.update(
            Update::Created {
                epoch: view.selected.epoch,
                result: Ok(session.clone().into()),
            },
            window,
            cx,
        );
        view.update(
            history_window(view.selected.epoch, session.into(), events),
            window,
            cx,
        );
        view.update(
            Update::Stream {
                epoch: view.selected.epoch,
                state: StreamState::Live,
            },
            window,
            cx,
        );
    });
}

#[gpui_kit::test]
fn first_prompt_receipt_locks_agent_and_rejection_keeps_it_selectable(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        finish_initialization(&fixture, meta.clone(), Vec::new(), window, cx);
        while fixture.commands.try_recv().is_ok() {}
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("first prompt", window, cx));
            view.send_prompt(window, cx);
            let epoch = view.selected.epoch;
            assert!(!view.can_select_agent());
            view.select_agent("other".into(), window, cx);
            assert_eq!(view.selected.id, Some(meta.id));
            assert_eq!(view.selected.epoch, epoch);
            let submission =
                submission::payload(fixture.commands.try_recv().expect("first prompt"));
            view.update(
                submission::rejected(meta.id, &submission, "rejected before acceptance"),
                window,
                cx,
            );
            assert!(view.can_select_agent());
            assert!(!view.thread_has_user_prompt());
            assert_eq!(view.composer.input.read(cx).value(), "first prompt");
            view.send_prompt(window, cx);
            let submission =
                submission::payload(fixture.commands.try_recv().expect("retry prompt"));
            view.update(submission::accepted_at(meta.id, &submission, 1), window, cx);
            assert!(view.thread_has_user_prompt());
            assert!(!view.can_select_agent());
            view.update(
                history_window(epoch, meta.clone().into(), Vec::new()),
                window,
                cx,
            );
            assert!(!view.can_select_agent());
        });
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("submission blocks switching and successful acceptance permanently fixes the agent");
}

#[gpui_kit::test]
fn initialized_agent_waits_for_complete_history_and_no_pending_drafts(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        finish_initialization(&fixture, meta.clone(), Vec::new(), window, cx);
        fixture.view.update(cx, |view, cx| {
            view.sessions
                .outboxes
                .entry(meta.id)
                .or_default()
                .enqueue(Draft {
                    text: "reserved prompt".into(),
                    attachments: Vec::new(),
                });
            assert!(!view.can_select_agent());
            view.sessions
                .outboxes
                .get_mut(&meta.id)
                .expect("outbox")
                .withdraw(0, Draft::default());
            assert!(view.can_select_agent());
            view.sessions.pending.archiving.insert(meta.id);
            assert!(!view.can_select_agent());
            view.sessions.pending.archiving.remove(&meta.id);
            view.selected.stream = StreamState::Loading;
            assert!(!view.can_select_agent());
            view.selected.stream = StreamState::Live;
            let mut newer = meta.clone();
            newer.revision = Cursor::from_raw(1);
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![newer.into()],
                })),
                window,
                cx,
            );
            assert!(!view.can_select_agent());
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                            bas::acp::ContentChunk::new(ContentBlock::from("startup banner")),
                        )),
                    )
                    .into(),
                },
                window,
                cx,
            );
            assert!(view.can_select_agent());
            view.update(
                history_window(
                    view.selected.epoch,
                    meta.clone().into(),
                    vec![Event::new(
                        meta.id,
                        Cursor::from_raw(8),
                        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                            bas::acp::ContentChunk::new(ContentBlock::from("retained tail")),
                        )),
                    )],
                ),
                window,
                cx,
            );
            assert!(view.thread_has_user_prompt());
            assert!(!view.can_select_agent());
        });
    })
    .expect("pending prompts and uncertain earlier history prevent switching");
}

#[gpui_kit::test]
fn pending_creation_disables_picker_and_rejects_late_selection(cx: &mut TestAppContext) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.window.compact = compact;
                view.resize_window(window, cx);
            });
            window.render_frame(cx);
            assert!(fixture.view.read(cx).selected.creating);
            assert!(!fixture.view.read(cx).can_select_agent());
            let windows_before = cx.windows();
            window.click("agent-picker", cx);
            assert!(fixture.view.read(cx).window.menu.is_none());
            assert_eq!(cx.windows(), windows_before);
            fixture.view.update(cx, |view, cx| {
                let epoch = view.selected.epoch;
                view.select_agent("other".into(), window, cx);
                assert_eq!(view.selected.epoch, epoch);
                assert_eq!(view.selected.agent, "embedded");
                assert!(view.selected.creating);
            });
            assert!(fixture.commands.try_recv().is_err());
        })
        .expect("pending creation cannot start a competing backend");
    }
}

#[gpui_kit::test]
fn stale_catalog_cannot_replace_the_agent_while_creating_or_waiting_for_login(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let previous = session();
    attach(&fixture, previous.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let new_epoch = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.start_agent_chat("new-agent".into(), window, cx);
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("new agent draft", window, cx)
                });
                let epoch = view.selected.epoch;
                let old_catalog = || {
                    Update::Catalog(Ok(connection::Catalog {
                        agents: vec![connection::AgentSummary {
                            id: "embedded".into(),
                            kind: "embedded".into(),
                            command: String::new(),
                        }],
                        sessions: vec![previous.clone().into()],
                    }))
                };
                view.update(old_catalog(), window, cx);
                assert_eq!(view.selected.agent, "new-agent");
                assert!(view.selected.creating);
                view.update(
                    Update::AuthenticationRequired {
                        epoch,
                        agent: "new-agent".into(),
                    },
                    window,
                    cx,
                );
                assert!(!view.selected.creating);
                assert_eq!(
                    view.selected.authentication.required.as_deref(),
                    Some("new-agent")
                );
                view.update(old_catalog(), window, cx);
                assert_eq!(view.selected.agent, "new-agent");
                assert_eq!(
                    view.selected.authentication.required.as_deref(),
                    Some("new-agent")
                );
                assert!(!view.selected.creating);
                assert!(
                    view.authentication_hint()
                        .expect("authentication message")
                        .contains("new-agent")
                );
                assert_eq!(view.composer.input.read(cx).value(), "new agent draft");
                epoch
            })
        })
        .expect("an older agent list cannot suppress a new agent's authentication request");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Create(request))
        if request.agent == "new-agent" && request.epoch == new_epoch)
    );
    assert!(fixture.commands.try_recv().is_err());
    cx.update(|cx| {
        cx.update_global::<crate::agents::Changes, _>(|changes, _| {
            changes.server = Some("127.0.0.1".parse().expect("fixture server"));
            changes.ready = Some("new-agent".into());
            changes.select = None;
            changes.revision += 1;
        });
    });
    cx.run_until_parked();
    let mut retry = None;
    while let Ok(command) = fixture.commands.try_recv() {
        match command {
            Command::Refresh => {}
            Command::Create(request) => {
                assert!(retry.is_none(), "login retries exactly once");
                assert_eq!(request.agent, "new-agent");
                assert!(request.epoch > new_epoch);
                retry = Some(request);
            }
            _ => panic!("login must retry the selected backend without sending a prompt"),
        }
    }
    let retry = retry.expect("login completion retries the newly installed agent");
    let mut created = session();
    created.agent = "new-agent".into();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: retry.epoch,
                    result: Ok(created.clone().into()),
                },
                window,
                cx,
            );
            assert_eq!(view.selected.id, Some(created.id));
            assert_eq!(
                view.current().expect("created session").agent.as_str(),
                "new-agent"
            );
            assert_eq!(view.composer.input.read(cx).value(), "new agent draft");
            view.update(
                history_window(view.selected.epoch, created.clone().into(), Vec::new()),
                window,
                cx,
            );
            assert_eq!(
                view.current().expect("restored session").agent.as_str(),
                "new-agent"
            );
            assert!(!view.selected.creating);
            assert!(view.selected.authentication.required.is_none());
        });
    })
    .expect("the successful retry retains the new backend and draft");
}
