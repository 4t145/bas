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

#[gpui_kit::test]
fn established_empty_session_locks_its_agent_in_both_modes(cx: &mut TestAppContext) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.resize_window(window, cx);
                view.composer.update(cx, |input, cx| {
                    input.set_value("keep this draft", window, cx)
                });
                assert!(
                    view.conversation
                        .as_ref()
                        .expect("empty session")
                        .items
                        .is_empty()
                );
            });
            window.render_frame(cx);
            let button = window.find("agent-picker");
            assert!(!fixture.view.read(cx).can_select_agent());
            assert_eq!(button.label(), Some("Agent · embedded"));
            let windows_before = cx.windows();
            window.click("agent-picker", cx);
            window.press("enter", cx);
            window.render_frame(cx);
            assert!(fixture.view.read(cx).menu.is_none());
            assert_eq!(cx.windows(), windows_before);
            fixture.view.update(cx, |view, cx| {
                view.select_agent("other".into(), window, cx);
                assert_eq!(view.active, Some(meta.id));
                assert_eq!(
                    view.current().expect("same session").agent.as_str(),
                    "embedded"
                );
                assert_eq!(view.selected_agent, "embedded");
                assert_eq!(view.composer.read(cx).value(), "keep this draft");
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
                view.compact = compact;
                view.resize_window(window, cx);
            });
            window.render_frame(cx);
            assert!(fixture.view.read(cx).creating);
            assert!(!fixture.view.read(cx).can_select_agent());
            let windows_before = cx.windows();
            window.click("agent-picker", cx);
            assert!(fixture.view.read(cx).menu.is_none());
            assert_eq!(cx.windows(), windows_before);
            fixture.view.update(cx, |view, cx| {
                let epoch = view.epoch;
                view.select_agent("other".into(), window, cx);
                assert_eq!(view.epoch, epoch);
                assert_eq!(view.selected_agent, "embedded");
                assert!(view.creating);
            });
            assert!(fixture.commands.try_recv().is_err());
        })
        .expect("pending creation cannot start a competing backend");
    }
}

#[gpui_kit::test]
fn explicit_new_thread_waits_for_agent_choice_across_catalog_refreshes(cx: &mut TestAppContext) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let previous = session();
        attach(&fixture, previous.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.resize_window(window, cx);
                view.composer.update(cx, |input, cx| {
                    input.set_value("old thread draft", window, cx)
                });
            });
            window.render_frame(cx);
            window.press("ctrl-n", cx);
            for _ in 0..3 {
                fixture.view.update(cx, |view, cx| {
                    view.update(
                        Update::Catalog(Ok(connection::Catalog {
                            agents: view.agents.clone(),
                            sessions: vec![previous.clone()],
                        })),
                        window,
                        cx,
                    );
                    view.refresh(cx);
                    assert!(view.waiting_for_agent_selection);
                    assert!(!view.creating);
                    assert!(view.can_select_agent());
                    assert_eq!(view.active, None);
                    assert_eq!(view.drafts[&previous.id].text, "old thread draft");
                });
            }
            window.render_frame(cx);
            assert!(window.find("agent-picker").visible());
        })
        .expect("new-thread action leaves a stable selectable draft");
        let commands: Vec<_> = std::iter::from_fn(|| fixture.commands.try_recv().ok()).collect();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::Detach))
        );
        assert!(commands.iter().all(|command| matches!(
            command,
            Command::Detach | Command::Refresh | Command::Foreground(_)
        )));
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("new thread draft", window, cx)
                });
                view.select_agent("codex-acp".into(), window, cx);
                assert!(!view.waiting_for_agent_selection);
                assert!(view.creating);
                assert!(!view.can_select_agent());
                assert_eq!(view.selected_agent, "codex-acp");
                assert_eq!(view.composer.read(cx).value(), "new thread draft");
            });
            window.render_frame(cx);
            window.click("agent-picker", cx);
            assert!(fixture.view.read(cx).menu.is_none());
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
fn uncreated_draft_can_choose_an_agent_and_keeps_its_text(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let popup = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fail_initialization(&fixture, window, cx);
            fixture.view.update(cx, |view, cx| {
                view.agents.push(pb::Agent {
                    id: "other".into(),
                    kind: "acp".into(),
                    command: String::new(),
                });
                view.compact = true;
                view.resize_window(window, cx);
                view.composer
                    .update(cx, |input, cx| input.set_value("saved text", window, cx));
            });
            window.render_frame(cx);
            assert!(fixture.view.read(cx).can_select_agent());
            window.click("agent-picker", cx);
            fixture
                .view
                .read(cx)
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
        assert_eq!(view.composer.read(cx).value(), "saved text");
        assert_eq!(view.selected_agent, "other");
        assert!(view.creating);
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
                view.agents.push(pb::Agent {
                    id: "other".into(),
                    kind: "acp".into(),
                    command: String::new(),
                });
                view.compact = true;
                view.resize_window(window, cx);
            });
            window.render_frame(cx);
            window.click("agent-picker", cx);
            fixture
                .view
                .read(cx)
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
        assert_eq!(view.active, Some(meta.id));
        assert_eq!(view.selected_agent, "embedded");
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
                view.composer.update(cx, |input, cx| {
                    input.set_value("new agent draft", window, cx)
                });
                let epoch = view.epoch;
                let old_catalog = || {
                    Update::Catalog(Ok(connection::Catalog {
                        agents: vec![pb::Agent {
                            id: "embedded".into(),
                            kind: "embedded".into(),
                            command: String::new(),
                        }],
                        sessions: vec![previous.clone()],
                    }))
                };
                view.update(old_catalog(), window, cx);
                assert_eq!(view.selected_agent, "new-agent");
                assert!(view.creating);
                view.update(
                    Update::AuthenticationRequired {
                        epoch,
                        agent: "new-agent".into(),
                    },
                    window,
                    cx,
                );
                assert!(!view.creating);
                assert_eq!(view.authentication_required.as_deref(), Some("new-agent"));
                view.update(old_catalog(), window, cx);
                assert_eq!(view.selected_agent, "new-agent");
                assert_eq!(view.authentication_required.as_deref(), Some("new-agent"));
                assert!(!view.creating);
                assert!(view.send_hint(cx).contains("new-agent"));
                assert_eq!(view.composer.read(cx).value(), "new agent draft");
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
        cx.update_global::<crate::settings::AgentChanges, _>(|changes, _| {
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
                    result: Ok(created.clone()),
                },
                window,
                cx,
            );
            assert_eq!(view.active, Some(created.id));
            assert_eq!(
                view.current().expect("created session").agent.as_str(),
                "new-agent"
            );
            assert_eq!(view.composer.read(cx).value(), "new agent draft");
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: created.clone(),
                    events: Vec::new(),
                },
                window,
                cx,
            );
            assert_eq!(
                view.current().expect("restored session").agent.as_str(),
                "new-agent"
            );
            assert!(!view.creating);
            assert!(view.authentication_required.is_none());
        });
    })
    .expect("the successful retry retains the new backend and draft");
}
