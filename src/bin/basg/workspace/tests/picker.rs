use super::*;

fn fail_initialization(fixture: &Fixture, window: &mut Window, cx: &mut App) {
    fixture.view.update(cx, |view, cx| {
        view.update(
            Update::Created {
                epoch: fixture.initial_create.epoch,
                result: Err("agent initialization failed".into()),
            },
            window,
            cx,
        );
    });
}

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
            Update::Snapshot {
                epoch: view.selected.epoch,
                session: session.into(),
                events,
            },
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
fn initialized_agent_with_startup_output_can_switch_and_preserve_composition(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        crate::agents::metadata::for_test(
            "127.0.0.1:7337".parse().expect("fixture server"),
            &bas::management::Catalog {
                entries: Vec::new(),
                warning: None,
                settings_path: None,
                can_install: false,
            },
            cx,
        );
    });
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        let file = tempfile::NamedTempFile::new().expect("attachment file");
        std::fs::write(file.path(), "draft attachment").expect("attachment content");
        let attachment = Attachment::load(file.path()).expect("draft attachment");
        let draft = format!("keep this draft {}", attachment.mention);
        let popup = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                finish_initialization(
                    &fixture,
                    meta.clone(),
                    vec![Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                            bas::acp::ContentChunk::new(ContentBlock::from("pi v0.87.1")),
                        )),
                    )],
                    window,
                    cx,
                );
                fixture.view.update(cx, |view, cx| {
                    view.backend.agents.push(connection::AgentSummary {
                        id: "other".into(),
                        kind: "acp".into(),
                        command: String::new(),
                    });
                    view.window.compact = compact;
                    view.resize_window(window, cx);
                    view.composer.expanded = !compact;
                    view.sync_composer_mode(cx);
                    view.composer.attachments = vec![attachment.clone()];
                    view.composer.input.update(cx, |input, cx| {
                        input.set_value(draft.clone(), window, cx);
                        input.set_selected_range(5..9, cx);
                        input.focus(window, cx);
                    });
                    assert!(view.can_select_agent());
                    assert!(!view.thread_has_user_prompt());
                });
                while fixture.commands.try_recv().is_ok() {}
                window.render_frame(cx);
                window.click("agent-picker", cx);
                compact.then(|| {
                    fixture
                        .view
                        .read(cx)
                        .window
                        .menu
                        .as_ref()
                        .expect("initialized agent menu")
                        .window
                        .expect("popup")
                })
            })
            .expect("initialized agent remains selectable");
        while fixture.commands.try_recv().is_ok() {}
        if let Some(popup) = popup {
            cx.update_window(popup, |_, window, cx| {
                window.render_frame(cx);
                assert_agent_menu_artwork(window, cx);
                window.within("popup-menu").click(1usize, cx);
            })
            .expect("switch agent through the compact popup");
        } else {
            cx.update_window(fixture.window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert_agent_menu_artwork(window, cx);
                window.press("down", cx);
                window.press("down", cx);
                window.press("enter", cx);
            })
            .expect("switch agent with the full-mode dropdown keyboard controls");
        }
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
        let Ok(Command::Create(request)) = fixture.commands.try_recv() else {
            panic!("switching creates a session for the chosen agent");
        };
        assert_eq!(request.agent, "other");
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                assert_eq!(view.selected.id, None);
                assert!(view.selected.creating);
                assert_eq!(view.selected.agent, "other");
                assert_eq!(view.composer.input.read(cx).value(), draft);
                assert_eq!(view.composer.input.read(cx).selected_range(), 5..9);
                assert_eq!(view.composer.attachments[0].id, attachment.id);
                assert_eq!(view.composer.expanded, !compact);
                assert!(
                    view.composer
                        .input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
                let old_epoch = request.epoch - 1;
                view.update(
                    Update::Event {
                        epoch: old_epoch,
                        event: Event::new(
                            meta.id,
                            Cursor::from_raw(2),
                            EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                                bas::acp::ContentChunk::new(ContentBlock::from("late banner")),
                            )),
                        ),
                    },
                    window,
                    cx,
                );
                view.update(
                    Update::Created {
                        epoch: old_epoch,
                        result: Err("late initialization failure".into()),
                    },
                    window,
                    cx,
                );
                assert!(view.selected.timeline.conversation.is_none());
                assert!(view.selected.error.is_none());
                assert_eq!(view.composer.input.read(cx).value(), draft);
            });
            let mut replacement = session();
            replacement.agent = "other".into();
            finish_initialization(&fixture, replacement, Vec::new(), window, cx);
            let view = fixture.view.read(cx);
            assert!(view.can_select_agent());
            assert_eq!(view.composer.input.read(cx).value(), draft);
            assert_eq!(view.composer.input.read(cx).selected_range(), 5..9);
            assert_eq!(view.composer.attachments[0].id, attachment.id);
            assert_eq!(view.composer.expanded, !compact);
            assert!(
                view.composer
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
        .expect("replacement initialization retains the draft and ignores stale updates");
    }
}

fn assert_agent_menu_artwork(window: &mut Window, cx: &App) {
    assert_eq!(
        window.find("agent-menu-label-embedded").label(),
        Some("embedded")
    );
    assert!(window.find("agent-menu-icon-embedded").visible());
    assert_eq!(
        crate::agents::icon::rendered_source("agent-menu-icon-embedded", cx).as_deref(),
        Some(crate::assets::OUTLINE)
    );
    assert!(window.find("agent-menu-icon-other-fallback").visible());
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
                Update::Snapshot {
                    epoch,
                    session: meta.clone().into(),
                    events: Vec::new(),
                },
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
fn user_or_turn_events_lock_an_initialized_agent_without_a_local_receipt(cx: &mut TestAppContext) {
    for kind in [
        EventKind::Acp(bas::acp::SessionUpdate::UserMessageChunk(
            bas::acp::ContentChunk::new(ContentBlock::from("external prompt")),
        )),
        EventKind::TurnStarted {
            turn: bas::model::TurnId::new(),
        },
        EventKind::TurnFinished {
            turn: bas::model::TurnId::new(),
            result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
        },
    ] {
        let fixture = fixture(cx);
        let meta = session();
        cx.update_window(fixture.window.into(), |_, window, cx| {
            finish_initialization(&fixture, meta.clone(), Vec::new(), window, cx);
            fixture.view.update(cx, |view, cx| {
                assert!(view.can_select_agent());
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: Event::new(meta.id, Cursor::from_raw(1), kind),
                    },
                    window,
                    cx,
                );
                assert!(view.thread_has_user_prompt());
                assert!(!view.can_select_agent());
            });
        })
        .expect("a remotely accepted or replayed turn fixes the session agent");
    }
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
                    ),
                },
                window,
                cx,
            );
            assert!(view.can_select_agent());
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: meta.clone().into(),
                    events: vec![Event::new(
                        meta.id,
                        Cursor::from_raw(8),
                        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                            bas::acp::ContentChunk::new(ContentBlock::from("retained tail")),
                        )),
                    )],
                },
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
fn complete_startup_history_reopens_an_unused_thread_but_cannot_unlock_an_accepted_prompt(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.revision = Cursor::from_raw(1);
    let banner = Event::new(
        meta.id,
        Cursor::from_raw(1),
        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
            bas::acp::ContentChunk::new(ContentBlock::from("startup only")),
        )),
    );
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(!view.can_select_agent());
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: meta.clone().into(),
                    events: vec![banner.clone()],
                },
                window,
                cx,
            );
            assert!(view.can_select_agent());
            assert!(!view.thread_has_user_prompt());
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(2),
                        EventKind::TurnStarted {
                            turn: bas::model::TurnId::new(),
                        },
                    ),
                },
                window,
                cx,
            );
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: meta.clone().into(),
                    events: vec![banner.clone()],
                },
                window,
                cx,
            );
            assert!(view.thread_has_user_prompt());
            assert!(!view.can_select_agent());
        });
    })
    .expect("complete replay proves an unused thread but cannot reverse confirmed acceptance");
}

#[gpui_kit::test]
fn established_empty_session_locks_its_agent_in_both_modes(cx: &mut TestAppContext) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.window.compact = compact;
                view.resize_window(window, cx);
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("keep this draft", window, cx)
                });
                assert!(
                    view.selected
                        .timeline
                        .conversation
                        .as_ref()
                        .expect("empty session")
                        .items
                        .is_empty()
                );
            });
            window.render_frame(cx);
            let button = window.find("agent-picker");
            assert!(!fixture.view.read(cx).can_select_agent());
            assert_eq!(button.label(), Some("智能体 · \u{2068}embedded\u{2069}"));
            let windows_before = cx.windows();
            window.click("agent-picker", cx);
            window.press("enter", cx);
            window.render_frame(cx);
            assert!(fixture.view.read(cx).window.menu.is_none());
            assert_eq!(cx.windows(), windows_before);
            fixture.view.update(cx, |view, cx| {
                view.select_agent("other".into(), window, cx);
                assert_eq!(view.selected.id, Some(meta.id));
                assert_eq!(
                    view.current().expect("same session").agent.as_str(),
                    "embedded"
                );
                assert_eq!(view.selected.agent, "embedded");
                assert_eq!(view.composer.input.read(cx).value(), "keep this draft");
            });
            assert!(fixture.commands.try_recv().is_err());
        })
        .expect("existing sessions keep their agent before the first prompt");
    }
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
fn explicit_new_thread_initializes_the_last_agent_once_across_catalog_refreshes(
    cx: &mut TestAppContext,
) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let previous = session();
        attach(&fixture, previous.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.window.compact = compact;
                view.resize_window(window, cx);
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("old thread draft", window, cx)
                });
            });
            window.render_frame(cx);
            window.press("ctrl-n", cx);
            for _ in 0..3 {
                fixture.view.update(cx, |view, cx| {
                    view.update(
                        Update::Catalog(Ok(connection::Catalog {
                            agents: view.backend.agents.clone(),
                            sessions: vec![previous.clone().into()],
                        })),
                        window,
                        cx,
                    );
                    view.refresh(cx);
                    assert!(!view.selected.waiting_for_agent_selection);
                    assert!(view.selected.creating);
                    assert!(!view.can_select_agent());
                    assert_eq!(view.selected.agent, previous.agent.to_string());
                    assert_eq!(view.selected.id, None);
                    assert_eq!(view.sessions.drafts[&previous.id].text, "old thread draft");
                });
            }
            window.render_frame(cx);
            assert!(window.find("agent-picker").visible());
        })
        .expect("new-thread action initializes the previous agent");
        let commands: Vec<_> = std::iter::from_fn(|| fixture.commands.try_recv().ok()).collect();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::Detach))
        );
        assert!(commands.iter().all(|command| matches!(
            command,
            Command::Detach | Command::Create(_) | Command::Refresh | Command::Foreground(_)
        )));
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, Command::Create(_)))
                .count(),
            1
        );
        cx.update_window(fixture.window.into(), |_, window, cx| {
            finish_initialization(&fixture, session(), vec![], window, cx);
            while fixture.commands.try_recv().is_ok() {}
            fixture.view.update(cx, |view, cx| {
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("new thread draft", window, cx)
                });
                view.select_agent("codex-acp".into(), window, cx);
                assert!(!view.selected.waiting_for_agent_selection);
                assert!(view.selected.creating);
                assert!(!view.can_select_agent());
                assert_eq!(view.selected.agent, "codex-acp");
                assert_eq!(view.composer.input.read(cx).value(), "new thread draft");
            });
            window.render_frame(cx);
            window.click("agent-picker", cx);
            assert!(fixture.view.read(cx).window.menu.is_none());
        })
        .expect("choice immediately initializes and locks the new agent");
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
        assert!(
            matches!(fixture.commands.try_recv(), Ok(Command::Create(request)) if request.agent == "codex-acp")
        );
        assert!(fixture.commands.try_recv().is_err());
    }
}

#[gpui_kit::test]
fn configured_default_wins_but_explicit_agent_chat_keeps_its_target(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        cx.update_global::<theme::Appearance, _>(|preferences, _| {
            preferences.default_agent = Some("preferred-agent".into());
        });
        fixture.view.update(cx, |view, cx| {
            view.new_thread(Some("/preferred".into()), window, cx);
            assert_eq!(view.selected.agent, "preferred-agent");
            assert!(view.selected.creating);
        });
    })
    .expect("configured default");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Create(request))
        if request.agent == "preferred-agent" && request.cwd == "/preferred")
    );
    assert!(fixture.commands.try_recv().is_err());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.start_agent_chat("explicit-agent".into(), window, cx);
            assert_eq!(view.selected.agent, "explicit-agent");
        });
    })
    .expect("explicit agent overrides default");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Create(request))
        if request.agent == "explicit-agent")
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn uncreated_draft_can_choose_an_agent_and_keeps_its_text(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let popup = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fail_initialization(&fixture, window, cx);
            fixture.view.update(cx, |view, cx| {
                view.backend.agents.push(connection::AgentSummary {
                    id: "other".into(),
                    kind: "acp".into(),
                    command: String::new(),
                });
                view.window.compact = true;
                view.resize_window(window, cx);
                view.composer
                    .input
                    .update(cx, |input, cx| input.set_value("saved text", window, cx));
            });
            window.render_frame(cx);
            assert!(fixture.view.read(cx).can_select_agent());
            window.click("agent-picker", cx);
            fixture
                .view
                .read(cx)
                .window
                .menu
                .as_ref()
                .expect("agent menu")
                .window
                .expect("popup")
        })
        .expect("uncreated draft permits selection");
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .expect("choose another backend");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Create(request)) if request.agent == "other")
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.composer.input.read(cx).value(), "saved text");
        assert_eq!(view.selected.agent, "other");
        assert!(view.selected.creating);
        assert!(!view.can_select_agent());
    })
    .expect("creation locks the chosen agent while retaining the draft");
}

#[gpui_kit::test]
fn menu_opened_for_a_draft_cannot_switch_a_newly_established_session(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let popup = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fail_initialization(&fixture, window, cx);
            fixture.view.update(cx, |view, cx| {
                view.backend.agents.push(connection::AgentSummary {
                    id: "other".into(),
                    kind: "acp".into(),
                    command: String::new(),
                });
                view.window.compact = true;
                view.resize_window(window, cx);
            });
            window.render_frame(cx);
            window.click("agent-picker", cx);
            fixture
                .view
                .read(cx)
                .window
                .menu
                .as_ref()
                .expect("draft menu")
                .window
                .expect("popup")
        })
        .expect("menu before session creation");
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .expect("delayed selection is rejected");
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(meta.id));
        assert_eq!(view.selected.agent, "embedded");
    });
    assert!(fixture.commands.try_recv().is_err());
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
            changes.server = Some("127.0.0.1:7337".parse().expect("fixture server"));
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
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: created.clone().into(),
                    events: Vec::new(),
                },
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
