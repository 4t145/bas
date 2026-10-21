use bas::model::{ContentBlock, Cursor, Event, EventKind, Owner, PromptSubmissionOutcome};
use gpui_kit::{Focusable, TestAppContext, WindowHandle, size, test::TestWindowExt};
use tokio::sync::mpsc;

use super::{navigation as workspace_navigation, *};

mod approval;
mod archive;
mod composer_queue;
mod connection_transition;
mod connections;
mod directories;
mod history;
mod interruption;
mod navigation;
mod operations;
mod paged_history;
mod picker;
mod read_state;
mod runtime;
mod submission;
mod thread_actions;
mod timeline;
mod tool_content;
mod workspace_removal;

struct Fixture {
    view: Entity<Workspace>,
    window: WindowHandle<Root>,
    commands: mpsc::UnboundedReceiver<Command>,
    initial_create: connection::CreateSession,
    _updates: mpsc::Sender<Update>,
    _appearance: tempfile::TempDir,
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    let appearance = tempfile::tempdir().expect("isolated appearance directory");
    let appearance_path = appearance.path().join("settings.json");
    std::fs::write(
        &appearance_path,
        r#"{"appearance":{"theme_source":"builtin","theme_mode":"dark"}}"#,
    )
    .expect("write deterministic appearance");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(appearance_path), cx);
        theme::init(None, cx);
        crate::application::init(cx);
        bind_keys(cx);
        crate::settings::window::bind_keys(cx);
    });
    let (commands, mut receiver) = mpsc::unbounded_channel();
    let (updates, events) = mpsc::channel(EVENT_BATCH);
    let connection = Connection::for_test(commands);
    let mut view = None;
    let window = cx.open_window(size(px(1120.), px(800.)), |window, cx| {
        let workspace = cx.new(|cx| {
            Workspace::with_connection(
                connection,
                events,
                "127.0.0.1".parse().expect("test address"),
                PathBuf::from("/tmp"),
                false,
                window,
                cx,
            )
        });
        workspace.update(cx, |view, cx| {
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: vec![connection::AgentSummary {
                        id: "embedded".into(),
                        kind: "embedded".into(),
                        command: String::new(),
                    }],
                    sessions: Vec::new(),
                })),
                window,
                cx,
            )
        });
        view = Some(workspace.clone());
        let host = crate::connections::workspace_view(workspace, window, cx);
        Root::new(host, window, cx)
    });
    let Command::Create(initial_create) = receiver.try_recv().expect("eager initialization") else {
        panic!("expected Create");
    };
    Fixture {
        initial_create,
        view: view.expect("workspace created"),
        window,
        commands: receiver,
        _updates: updates,
        _appearance: appearance,
    }
}

fn session() -> SessionMeta {
    let mut session = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    session.status = SessionStatus::Idle;
    session
}

fn history_window(epoch: u64, session: bas::model::SessionSnapshot, events: Vec<Event>) -> Update {
    Update::HistoryWindow {
        epoch,
        window: bas::service::HistoryWindow {
            session,
            cursor: events.last().map_or(0, |event| event.cursor.raw()),
            events,
            context: Vec::new(),
            before: None,
        },
        cached: false,
    }
}

#[gpui_kit::test]
fn title_and_background_catalog_wait_for_receipt_correlated_tracking(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let accepted = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer
                    .input
                    .update(cx, |input, cx| input.set_value("A", window, cx));
                view.send_prompt(window, cx);
                let command = fixture.commands.try_recv().expect("A");
                assert!(matches!(&command, Command::Prompt { submission, .. } if submission::text(submission) == "A"));
                let submission = submission::payload(command);
                view.sessions
                    .outboxes
                    .get_mut(&first.id)
                    .expect("outbox")
                    .enqueue(Draft {
                        text: "B".into(),
                        attachments: Vec::new(),
                    });
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: Event::new(
                            first.id,
                            Cursor::from_raw(2),
                            EventKind::TitleChanged {
                                title: "renamed".into(),
                            },
                        ),
                    },
                    window,
                    cx,
                );
                view.update(
                    submission::accepted_at(first.id, &submission, 3),
                    window,
                    cx,
                );
                assert!(fixture.commands.try_recv().is_err());
                bas::model::TurnReceipt {
                    turn: submission.turn,
                    cursor: Cursor::from_raw(3),
                }
            })
        })
        .expect("window");
    attach(&fixture, second.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let mut completed = first.clone();
            completed.revision = Cursor::from_raw(4);
            view.update(Update::Catalog(Ok(connection::Catalog {
                agents: view.backend.agents.clone(), sessions: vec![completed.into(), second.into()],
            })), window, cx);
            assert!(fixture.commands.try_recv().is_err());
            view.update(Update::TrackedTurn {
                event: Event::new(first.id, Cursor::from_raw(4), EventKind::TurnFinished {
                    turn: accepted.turn,
                    result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
                }),
            }, window, cx);
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session, submission, .. }) if session == first.id && submission::text(&submission) == "B"));
    }).expect("window");
}

#[gpui_kit::test]
fn catalog_and_history_merge_metadata_without_skipping_content(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut newest = session();
    newest.revision = Cursor::from_raw(10);
    newest.title = Some("latest".into());
    newest.status = SessionStatus::Working;
    newest.active_turn = Some(bas::model::TurnId::new());
    attach(&fixture, newest.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let mut old = newest.clone();
            old.revision = Cursor::from_raw(2);
            old.title = Some("old".into());
            old.status = SessionStatus::Idle;
            old.active_turn = None;
            let finish = Event::new(
                newest.id,
                Cursor::from_raw(5),
                EventKind::TurnFinished {
                    turn: bas::model::TurnId::new(),
                    result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
                },
            );
            view.update(
                history_window(
                    view.selected.epoch,
                    old.clone().into(),
                    vec![
                        Event::new(
                            newest.id,
                            Cursor::from_raw(4),
                            EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                                bas::acp::ContentChunk::new("history".into()),
                            )),
                        ),
                        finish,
                    ],
                ),
                window,
                cx,
            );
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![old.into()],
                })),
                window,
                cx,
            );
            let conversation = view
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("conversation");
            assert_eq!(conversation.session, newest);
            assert_eq!(conversation.cursor, 5);
            assert_eq!(conversation.items.len(), 1);
            assert_eq!(view.sessions.entries[0], newest);
            // A completion between history and GetSession stays newer than the snapshot.
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        newest.id,
                        Cursor::from_raw(11),
                        EventKind::TurnFinished {
                            turn: newest.active_turn.expect("turn"),
                            result: Ok(bas::acp::PromptResponse::new(
                                bas::acp::StopReason::EndTurn,
                            )),
                        },
                    ),
                },
                window,
                cx,
            );
            assert_eq!(view.current().expect("current").status, SessionStatus::Idle);
        });
    })
    .expect("window");
}

#[gpui_kit::test]
fn editing_queue_head_keeps_order_and_escape_restores_composition(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.enqueue_draft(meta.id, "A".into(), window, cx);
            view.enqueue_draft(meta.id, "B".into(), window, cx);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("composition", window, cx));
        });
        window.render_frame(cx);
        window.click("pending-messages", cx);
        assert_eq!(fixture.view.read(cx).composer.input.read(cx).value(), "A");
        window.press("escape", cx);
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "composition"
        );
        window.render_frame(cx);
        window.click("pending-messages", cx);
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("A'", window, cx));
            view.send_prompt(window, cx);
            assert_eq!(view.composer.input.read(cx).value(), "composition");
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::TurnFinished {
                            turn: bas::model::TurnId::new(),
                            result: Ok(bas::acp::PromptResponse::new(
                                bas::acp::StopReason::EndTurn,
                            )),
                        },
                    ),
                },
                window,
                cx,
            );
        });
        assert!(
            matches!(fixture.commands.try_recv(), Ok(Command::Prompt { submission, .. }) if submission::text(&submission) == "A'")
        );
        assert_eq!(
            fixture.view.read(cx).sessions.outboxes[&meta.id]
                .front()
                .expect("B")
                .text,
            "B"
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn receipt_after_session_deletion_cannot_recreate_outbox_or_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Catalog(Ok(crate::connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![meta.clone().into()],
                })),
                window,
                cx,
            );
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("A", window, cx));
            view.send_prompt(window, cx);
            let submission = submission::payload(fixture.commands.try_recv().expect("A"));
            view.update(
                Update::Catalog(Ok(crate::connection::Catalog {
                    agents: Vec::new(),
                    sessions: Vec::new(),
                })),
                window,
                cx,
            );
            view.update(
                submission::rejected(meta.id, &submission, "late failure"),
                window,
                cx,
            );
            assert!(!view.sessions.outboxes.contains_key(&meta.id));
            assert!(!view.sessions.drafts.contains_key(&meta.id));
        });
    })
    .expect("window");
}

fn attach(fixture: &Fixture, session: SessionMeta, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries.push(session.clone());
            view.select(session.id, window, cx);
            view.update(
                history_window(view.selected.epoch, session.into(), Vec::new()),
                window,
                cx,
            );
            view.selected.stream = StreamState::Live;
            cx.notify();
        });
    })
    .expect("test window available");
}

#[gpui_kit::test]
fn sending_waits_for_connection_and_session_readiness(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("保留这份草稿", cx);
        for (connected, error, stream, creating, expected) in [
            (
                false,
                None,
                StreamState::Loading,
                false,
                status::ConnectionStatus::Connecting,
            ),
            (
                true,
                None,
                StreamState::Loading,
                false,
                status::ConnectionStatus::Connecting,
            ),
            (
                true,
                None,
                StreamState::Reconnecting("retry".into()),
                false,
                status::ConnectionStatus::Connecting,
            ),
            (
                false,
                Some("offline".into()),
                StreamState::Live,
                false,
                status::ConnectionStatus::Disconnected,
            ),
            (
                true,
                None,
                StreamState::Unavailable("gone".into()),
                false,
                status::ConnectionStatus::Disconnected,
            ),
            (
                true,
                None,
                StreamState::Closed,
                false,
                status::ConnectionStatus::Disconnected,
            ),
            (
                true,
                None,
                StreamState::Live,
                true,
                status::ConnectionStatus::Connected,
            ),
        ] {
            fixture.view.update(cx, |view, cx| {
                view.backend.connected = connected;
                view.backend.error = error;
                view.selected.stream = stream;
                view.selected.creating = creating;
                assert_eq!(view.connection_status(), expected);
                assert!(!view.can_send(cx));
                cx.notify();
            });
            window.render_frame(cx);
            window.click("send-message", cx);
            fixture
                .view
                .read(cx)
                .composer
                .input
                .clone()
                .update(cx, |input, cx| input.focus(window, cx));
            window.press("enter", cx);
            assert!(
                fixture.commands.try_recv().is_err(),
                "unready connections must not send"
            );
            assert_eq!(
                fixture.view.read(cx).composer.input.read(cx).value(),
                "保留这份草稿"
            );
        }
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = true;
            view.backend.error = None;
            view.selected.stream = StreamState::Live;
            view.selected.creating = false;
            assert!(view.can_send(cx));
            assert!(
                view.activity_label().is_none(),
                "idle connection status is icon-only"
            );
            cx.notify();
        });
        window.render_frame(cx);
        window.click("send-message", cx);
        match fixture.commands.try_recv() {
            Ok(Command::Prompt { submission, .. }) => {
                assert_eq!(submission::text(&submission), "保留这份草稿")
            }
            _ => panic!("ready send button must dispatch the saved draft"),
        }
    })
    .expect("window");
}

#[gpui_kit::test]
fn selecting_an_agent_initializes_options_before_any_prompt(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    assert_eq!(fixture.initial_create.agent, "embedded");
    assert!(fixture.initial_create.title.is_empty());
    assert_eq!(fixture.initial_create.cwd, "/tmp");
    let mut created = session();
    created.cwd = Some(PathBuf::from("/tmp"));
    created.acp = Some(
        bas::acp::NewSessionResponse::new(created.id.to_string()).config_options(vec![
            serde_json::from_value(serde_json::json!({
                "id": "thinking", "name": "思考", "type": "boolean", "currentValue": false
            }))
            .expect("ACP boolean option"),
        ]),
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("代码审查", cx);
        window.press("ctrl-enter", cx);
    })
    .expect("window");
    assert!(
        fixture.commands.try_recv().is_err(),
        "initialization must not send a prompt"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.composer.input.read(cx).value(), "代码审查");
            let cursor = view.composer.input.read(cx).cursor_position();
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(created.clone().into()),
                },
                window,
                cx,
            );
            assert_eq!(view.composer.input.read(cx).cursor_position(), cursor);
            assert_eq!(view.selected.id, Some(created.id));
            assert_eq!(view.composer.input.read(cx).value(), "代码审查");
            view.update(
                history_window(view.selected.epoch, created.clone().into(), vec![]),
                window,
                cx,
            );
            view.selected.stream = StreamState::Live;
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("config-thinking").visible());
        window.click("config-thinking", cx);
        window.press("ctrl-shift-m", cx);
        window.render_frame(cx);
        assert!(window.find("config-thinking").visible());
        assert!(window.find("agent-picker").visible());
    })
    .expect("test window available");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == created.id)
    );
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::SetConfig(request)) if request.session_id.to_string() == created.id.to_string())
    );
    assert!(
        fixture.commands.try_recv().is_err(),
        "initialization must not send a prompt"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(
                !view.can_send(cx),
                "wait for the option change before starting a turn"
            );
            view.update(
                Update::Configured {
                    session: created.id,
                    result: Ok(bas::acp::SetSessionConfigOptionResponse::new(vec![])),
                },
                window,
                cx,
            );
        });
        window.press("ctrl-enter", cx);
    })
    .expect("window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session, submission, .. }) if session == created.id && submission::text(&submission) == "代码审查")
    );
}

#[gpui_kit::test]
fn modes_and_output_fold_preserve_the_thread_and_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let _ = fixture.commands.try_recv();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("保留草稿", cx);
        window.press("ctrl-shift-m", cx);
        window.render_frame(cx);
        assert!(window.find("toggle-output").visible());
        assert!(window.try_find("add-workspace-button").is_none());
        assert!(fixture.view.read(cx).window.compact);
        assert!(!fixture.view.read(cx).window.output_open);
        window.click("toggle-output", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).window.output_open);
        window.press("ctrl-o", cx);
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).window.output_open);
        window.press("ctrl-shift-m", cx);
        window.render_frame(cx);
        assert!(window.find("add-workspace-button").visible());
        assert!(window.try_find("toggle-output").is_none());
        assert!(!fixture.view.read(cx).window.compact);
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.selected.id, Some(session.id));
            assert_eq!(view.composer.input.read(cx).value(), "保留草稿");
            assert!(view.selected.timeline.conversation.is_some());
        });
    })
    .expect("test window available");
    assert!(
        fixture.commands.try_recv().is_err(),
        "layout changes must not recreate or detach sessions"
    );
}

#[gpui_kit::test]
fn late_initialization_cannot_steal_another_agent_or_workspace(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let created = session();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("保留草稿", cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Err("initialization failed before choosing another backend".into()),
                },
                window,
                cx,
            );
            view.select_agent("other-agent".into(), window, cx);
            assert_eq!(view.composer.input.read(cx).value(), "保留草稿");
            view.new_thread(Some(PathBuf::from("/other")), window, cx);
            assert_eq!(view.selected.agent, "other-agent");
        });
        window.input("另一工作区草稿", cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(created.clone().into()),
                },
                window,
                cx,
            );
            assert_eq!(
                view.navigation.selected_directory.as_deref(),
                Some(std::path::Path::new("/other"))
            );
            assert_eq!(view.selected.id, None);
            assert!(view.selected.creating);
            assert_eq!(view.composer.input.read(cx).value(), "另一工作区草稿");
        });
    })
    .expect("test window available");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Create(form)) if form.agent == "other-agent" && form.cwd == "/tmp")
    );
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
    let Command::Create(form) = fixture
        .commands
        .try_recv()
        .expect("new directory initialized")
    else {
        panic!("Create");
    };
    assert_eq!(form.cwd, "/other");
    assert!(fixture.commands.try_recv().is_err());
    let mut latest = session();
    latest.cwd = Some(PathBuf::from("/other"));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: form.epoch,
                    result: Ok(latest.clone().into()),
                },
                window,
                cx,
            );
            assert_eq!(view.selected.id, Some(latest.id));
            assert_eq!(view.composer.input.read(cx).value(), "另一工作区草稿");
        });
    })
    .expect("window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == latest.id)
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn late_events_and_send_receipts_cannot_modify_another_session(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("第一条消息", cx);
        window.press("enter", cx);
    })
    .expect("test window available");
    let old_epoch = cx.update(|cx| fixture.view.read(cx).selected.epoch);
    let submission = submission::payload(fixture.commands.try_recv().expect("first prompt"));
    attach(&fixture, second.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("另一条草稿", cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                submission::accepted_at(first.id, &submission, 1),
                window,
                cx,
            );
            view.update(
                Update::Event {
                    epoch: old_epoch,
                    event: Event::new(
                        first.id,
                        Cursor::from_raw(99),
                        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                            bas::acp::ContentChunk::new(ContentBlock::from("迟到的回复")),
                        )),
                    ),
                },
                window,
                cx,
            );
            assert_eq!(view.composer.input.read(cx).value(), "另一条草稿");
            assert!(
                view.selected
                    .timeline
                    .conversation
                    .as_ref()
                    .expect("second session")
                    .items
                    .is_empty()
            );
            assert_eq!(view.selected.id, Some(second.id));
            view.select(first.id, window, cx);
            assert!(view.composer.input.read(cx).value().is_empty());
            view.select(second.id, window, cx);
            assert_eq!(view.composer.input.read(cx).value(), "另一条草稿");
        });
    })
    .expect("test window available");
    // Drain commands so the test also ensures the fake connection stays alive.
    while fixture.commands.try_recv().is_ok() {}
}

#[gpui_kit::test]
fn failed_initialization_keeps_draft_and_can_be_retried(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("尚未发送", cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            assert!(!view.selected.creating);
            assert_eq!(view.composer.input.read(cx).value(), "尚未发送");
        });
        window.press("f5", cx);
    })
    .expect("window");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Refresh)));
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::Create(_))
    ));
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn attachments_follow_drafts_and_survive_failed_send(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut first = session();
    first.initialization =
        Some(
            bas::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
                .agent_capabilities(bas::acp::AgentCapabilities::new().prompt_capabilities(
                    bas::acp::PromptCapabilities::new().embedded_context(true),
                )),
        );
    let second = session();
    attach(&fixture, first.clone(), cx);
    let file = tempfile::NamedTempFile::new().expect("file");
    std::fs::write(file.path(), "附件内容").expect("contents");
    let attachment = Attachment::load(file.path()).expect("attachment");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.add_attachments(first.id, vec![attachment.clone()], window, cx);
            assert!(view.can_send(cx), "attachments can be sent without text");
            view.sessions.entries.push(second.clone());
            view.select(second.id, window, cx);
            assert!(view.composer.attachments.is_empty());
            view.select(first.id, window, cx);
            assert_eq!(view.composer.attachments[0].id, attachment.id);
            view.selected.stream = StreamState::Live;
            while fixture.commands.try_recv().is_ok() {}
            view.send_prompt(window, cx);
            let command = fixture.commands.try_recv().expect("attachment prompt");
            let submission = submission::payload(command);
            assert_eq!(
                submission.request.prompt,
                crate::attachment::prompt_content(
                    &format!("{} ", attachment.mention),
                    std::slice::from_ref(&attachment),
                    &view.prompt_capabilities(),
                ),
            );
            view.update(
                submission::rejected(first.id, &submission, "offline"),
                window,
                cx,
            );
            assert_eq!(view.composer.attachments[0].id, attachment.id);
            view.send_prompt(window, cx);
            let submission =
                submission::payload(fixture.commands.try_recv().expect("retry attachment"));
            view.update(
                submission::accepted_at(first.id, &submission, 1),
                window,
                cx,
            );
            assert!(view.composer.attachments.is_empty());
        });
    })
    .expect("window");
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn terminal_reference_does_not_hide_raw_tool_result(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let tool = bas::acp::ToolCall::new("shell", "fastfetch")
                .content(vec![bas::acp::ToolCallContent::Terminal(
                    bas::acp::Terminal::new("remote"),
                )])
                .raw_output(serde_json::json!("Linux 6.x\n结果"));
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(bas::acp::SessionUpdate::ToolCall(tool)),
                    ),
                },
                window,
                cx,
            );
            view.selected.timeline.expanded.insert("tool-shell".into());
        });
        window.render_frame(cx);
        assert!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .markdown
                .borrow()
                .contains_key("tool-shell-output"),
            "raw output must remain visible when a terminal reference has no stream"
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn clipboard_images_are_inline_and_support_delete_undo_and_text_paste(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut session = session();
    session.initialization = Some(
        bas::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .agent_capabilities(
                bas::acp::AgentCapabilities::new()
                    .prompt_capabilities(bas::acp::PromptCapabilities::new().image(true)),
            ),
    );
    attach(&fixture, session, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("前后", cx);
        let composer = fixture.view.read(cx).composer.input.clone();
        composer.update(cx, |input, cx| input.set_selected_range(3..3, cx));
        let image =
            gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b"\x89PNG\r\n\x1a\n".to_vec());
        cx.write_to_clipboard(ClipboardItem::new_image(&image));
        window.press("ctrl-v", cx);
        window.render_frame(cx);
        let value = fixture.view.read(cx).composer.input.read(cx).value();
        let attachment = fixture.view.read(cx).composer.attachments[0].clone();
        assert_eq!(value, format!("前{} 后", attachment.mention));
        assert_eq!(attachment.label, "图片");
        assert!(
            window
                .try_find(SharedString::from(format!(
                    "attachment-{}-3",
                    attachment.id
                )))
                .is_some()
        );
        assert!(window.try_find("attachments").is_none());
        let view = fixture.view.read(cx);
        let blocks = crate::attachment::prompt_content(
            &value,
            &view.composer.attachments,
            &view.prompt_capabilities(),
        );
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[1], bas::acp::ContentBlock::Image(_)));
        let end = "前".len() + attachment.mention.len();
        composer.update(cx, |input, cx| input.set_selected_range(end..end, cx));
        window.press("backspace", cx);
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "前 后"
        );
        window.press("ctrl-z", cx);
        assert_eq!(fixture.view.read(cx).composer.input.read(cx).value(), value);
        cx.write_to_clipboard(ClipboardItem::new_string("普通文本".into()));
        window.press("ctrl-v", cx);
        assert!(
            fixture
                .view
                .read(cx)
                .composer
                .input
                .read(cx)
                .value()
                .contains("普通文本")
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn unsupported_attachment_is_rejected_before_insertion_and_hint_clears_on_edit(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("binary.dat");
    std::fs::write(&path, b"\0\xff\x00").expect("binary");
    let binary = Attachment::load(&path).expect("attachment");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("原有草稿", cx);
    })
    .expect("window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.add_attachments(session.id, vec![binary], window, cx);
            assert!(view.composer.attachments.is_empty());
            assert_eq!(view.composer.input.read(cx).value(), "原有草稿");
            assert!(
                view.composer_error(cx)
                    .expect("attachment error")
                    .starts_with("请为此智能体选择其他附件类型")
            );
            assert_eq!(
                view.stream_label(),
                "已同步",
                "operation failures must not replace status"
            );
        });
    })
    .expect("window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.input("继续编辑", cx);
    })
    .expect("window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert!(fixture.view.read(cx).selected.error.is_none());
        assert!(
            fixture
                .view
                .read(cx)
                .send_hint(cx)
                .starts_with("发送消息 · ")
        );
        let path = directory.path().join("flake.nix");
        std::fs::write(&path, "{ inputs = {}; }\n").expect("Nix");
        fixture.view.update(cx, |view, cx| {
            view.add_attachments(
                session.id,
                vec![Attachment::load(&path).expect("Nix")],
                window,
                cx,
            );
            assert_eq!(view.composer.attachments.len(), 1);
            assert!(view.can_send(cx));
            assert!(view.selected.error.is_none());
        });
        window.render_frame(cx);
        assert!(window.find("add-attachment").visible());
    })
    .expect("window");
}

#[gpui_kit::test]
fn rejected_png_restores_editing_and_does_not_block_a_text_prompt(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let image =
        gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b"\x89PNG\r\n\x1a\n".to_vec());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("草稿", cx);
        fixture.view.update(cx, |view, cx| {
            view.navigation
                .search
                .update(cx, |input, cx| input.focus(window, cx));
            view.add_attachments(
                session.id,
                vec![Attachment::image(&image).expect("PNG")],
                window,
                cx,
            );
            assert!(
                view.composer
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert!(view.composer.attachments.is_empty());
            assert_eq!(view.composer.input.read(cx).value(), "草稿");
            assert!(view.can_send(cx));
        });
        window.press("escape", cx);
        assert!(fixture.view.read(cx).selected.error.is_none());
        window.press("backspace", cx);
        assert_eq!(fixture.view.read(cx).composer.input.read(cx).value(), "草");
        window.press("enter", cx);
    })
    .expect("window");
    let mut sent = false;
    // The foreground update may accompany a render or activation.
    while let Ok(command) = fixture.commands.try_recv() {
        if let Command::Prompt { submission, .. } = command {
            assert_eq!(submission::text(&submission), "草");
            assert_eq!(submission.request.prompt.len(), 1);
            sent = true;
        }
    }
    assert!(sent);
}

#[gpui_kit::test]
fn foreground_presence_tracks_window_activation_and_selected_thread(cx: &mut TestAppContext) {
    let mut primary = fixture(cx);
    let first = session();
    attach(&primary, first.clone(), cx);
    while primary.commands.try_recv().is_ok() {}
    cx.update_window(primary.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("activate window");
    cx.executor().run_until_parked();
    let mut presence = Vec::new();
    while let Ok(command) = primary.commands.try_recv() {
        if let Command::Foreground(session) = command {
            presence.push(session);
        }
    }
    assert_eq!(presence, vec![Some(first.id)]);
    let second = session();
    attach(&primary, second.clone(), cx);
    cx.update_window(primary.window.into(), |_, window, cx| {
        window.render_frame(cx)
    })
    .expect("selected thread");
    let other_window = fixture(cx);
    cx.update_window(other_window.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("activate other window");
    cx.executor().run_until_parked();
    presence.clear();
    while let Ok(command) = primary.commands.try_recv() {
        if let Command::Foreground(session) = command {
            presence.push(session);
        }
    }
    assert_eq!(presence, vec![Some(second.id), None]);
}

#[gpui_kit::test]
fn folded_reply_stays_unread_until_opened_and_survives_replay(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, _| {
        window.activate_window()
    })
    .expect("activate reply window");
    cx.executor().run_until_parked();
    let events = completed_events(session.id, "完成检查，中英文 Hello");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        fixture.view.update(cx, |view, cx| {
            view.window.compact = true;
            view.window.output_open = false;
            view.resize_window(window, cx);
            for event in &events {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: event.clone(),
                    },
                    window,
                    cx,
                );
            }
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        assert!(
            !fixture
                .view
                .read(cx)
                .sessions
                .seen_replies
                .contains_key(&session.id)
        );
        assert!(window.find("unread-reply").bounds().size.width > px(0.));
        let trigger = window.find("completed-reply").bounds();
        fixture.view.update(cx, |view, cx| {
            view.open_reply_preview(trigger, window, cx);
            assert!(view.window.reply_preview.is_some());
            assert!(
                !view.sessions.seen_replies.contains_key(&session.id),
                "preview must not consume unread status"
            );
        });
        assert!(window.is_window_active(), "preview must not take focus");
        window.click("completed-reply", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).window.output_open);
        assert!(fixture.view.read(cx).window.reply_preview.is_none());
        assert_eq!(
            fixture.view.read(cx).sessions.seen_replies.get(&session.id),
            Some(&2)
        );
        fixture.view.update(cx, |view, cx| {
            view.toggle_output(window, cx);
            view.update(
                history_window(view.selected.epoch, session.clone().into(), events),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(
            fixture.view.read(cx).sessions.seen_replies.get(&session.id),
            Some(&2)
        );
        assert!(!fixture.view.read(cx).window.output_open);
    })
    .expect("reply window");
}

fn completed_events(session: SessionId, text: &str) -> Vec<Event> {
    let turn = bas::model::TurnId::new();
    [
        EventKind::TurnStarted { turn },
        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
            bas::acp::ContentChunk::new(text.into()),
        )),
        EventKind::TurnFinished {
            turn,
            result: Ok(bas::acp::PromptResponse::new(
                bas::model::StopReason::EndTurn,
            )),
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| Event::new(session, Cursor::from_raw(index as u64 + 1), kind))
    .collect()
}

#[gpui_kit::test]
fn unread_reply_is_scoped_to_its_session_and_requires_a_visible_foreground_answer(
    cx: &mut TestAppContext,
) {
    let primary = fixture(cx);
    let first = session();
    attach(&primary, first.clone(), cx);
    let other = fixture(cx);
    cx.update_window(other.window.into(), |_, window, _| window.activate_window())
        .expect("background primary");
    cx.executor().run_until_parked();
    cx.update_window(primary.window.into(), |_, window, cx| {
        primary.view.update(cx, |view, cx| {
            for event in completed_events(first.id, &"一段较长的回复。\n\n".repeat(100)) {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event,
                    },
                    window,
                    cx,
                );
            }
            view.selected
                .timeline
                .list
                .set_follow_mode(FollowMode::Normal);
            view.selected
                .timeline
                .list
                .scroll_to(gpui_kit::ListOffset::default());
        });
        window.render_frame(cx);
        assert!(
            !primary
                .view
                .read(cx)
                .sessions
                .seen_replies
                .contains_key(&first.id)
        );
    })
    .expect("background response");
    cx.executor().run_until_parked();
    cx.update_window(primary.window.into(), |_, window, _| {
        window.activate_window()
    })
    .expect("activate primary");
    cx.executor().run_until_parked();
    cx.update_window(primary.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            !primary
                .view
                .read(cx)
                .sessions
                .seen_replies
                .contains_key(&first.id),
            "the final answer is below the viewport"
        );
        window.click("scroll-to-bottom", cx);
        window.render_frame(cx);
        assert_eq!(
            primary.view.read(cx).sessions.seen_replies.get(&first.id),
            Some(&2)
        );
    })
    .expect("read latest answer");
    let second = session();
    attach(&primary, second.clone(), cx);
    cx.update_window(primary.window.into(), |_, window, cx| {
        primary.view.update(cx, |view, cx| {
            view.window.compact = true;
            view.window.output_open = false;
            for event in completed_events(second.id, "另一个会话的新回复") {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event,
                    },
                    window,
                    cx,
                );
            }
        });
        window.render_frame(cx);
        assert!(
            !primary
                .view
                .read(cx)
                .sessions
                .seen_replies
                .contains_key(&second.id)
        );
        assert_eq!(
            primary.view.read(cx).sessions.seen_replies.get(&first.id),
            Some(&2)
        );
    })
    .expect("separate unread state");
}

#[gpui_kit::test]
fn running_turn_queues_messages_in_order_and_keeps_failed_messages(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut session = session();
    session.status = SessionStatus::Working;
    attach(&fixture, session.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    for text in ["追加一", "追加二"] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.input(text, cx);
            window.press("enter", cx);
        })
        .expect("enqueue from composer");
    }
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert!(
            fixture.commands.try_recv().is_err(),
            "running turns must not overlap"
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .sessions
                .outboxes
                .get(&session.id)
                .map(|q| q.len()),
            Some(2),
            "text after enter: {:?}",
            fixture.view.read(cx).composer.input.read(cx).value()
        );
        assert!(
            fixture
                .view
                .read(cx)
                .composer
                .input
                .read(cx)
                .value()
                .is_empty()
        );
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::TurnFinished {
                            turn: bas::model::TurnId::new(),
                            result: Ok(bas::acp::PromptResponse::new(
                                bas::acp::StopReason::EndTurn,
                            )),
                        },
                    ),
                },
                window,
                cx,
            );
        });
        let command = fixture.commands.try_recv().expect("first queued prompt");
        assert!(matches!(&command, Command::Prompt { submission, .. } if submission::text(submission) == "追加一"));
        let submission = submission::payload(command);
        let accepted = bas::model::TurnReceipt {
            turn: submission.turn,
            cursor: Cursor::from_raw(2),
        };
        fixture.view.update(cx, |view, cx| {
            view.update(
                submission::accepted_at(session.id, &submission, 2),
                window,
                cx,
            );
        });
        assert!(
            fixture.commands.try_recv().is_err(),
            "receipt must not drain the next prompt using a stale Idle snapshot"
        );
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(2),
                        EventKind::TurnStarted {
                            turn: accepted.turn,
                        },
                    ),
                },
                window,
                cx,
            );
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(3),
                        EventKind::TurnFinished {
                            turn: accepted.turn,
                            result: Ok(bas::acp::PromptResponse::new(
                                bas::acp::StopReason::EndTurn,
                            )),
                        },
                    ),
                },
                window,
                cx,
            );
        });
        let command = fixture.commands.try_recv().expect("second queued prompt");
        assert!(matches!(&command, Command::Prompt { submission, .. } if submission::text(submission) == "追加二"));
        let submission = submission::payload(command);
        fixture.view.update(cx, |view, cx| {
            view.update(
                submission::rejected(session.id, &submission, "offline"),
                window,
                cx,
            );
            view.flush_queue(cx);
        });
        assert!(
            fixture.commands.try_recv().is_err(),
            "failed queued sends must wait for user retry"
        );
        window.render_frame(cx);
        window.click("pending-messages", cx);
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "追加二"
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn external_file_drop_and_file_clipboard_insert_native_attachments(cx: &mut TestAppContext) {
    use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session, cx);
    let directory = tempfile::tempdir().expect("attachment directory");
    let path = directory.path().join("settings.nix");
    std::fs::write(&path, "{ enabled = true; }").expect("attachment file");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("composer-panel").bounds().center();
        window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Entered {
                position,
                paths: ExternalPaths([path.clone()].into_iter().collect()),
            }),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Submit { position }),
            cx,
        );
    })
    .expect("window");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(
            crate::attachment::mentions(
                &view.composer.input.read(cx).value(),
                &view.composer.attachments
            )
            .len(),
            1
        );
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![gpui_kit::ClipboardEntry::ExternalPaths(ExternalPaths(
                [path].into_iter().collect(),
            ))],
        });
        window.press("ctrl-v", cx);
    })
    .expect("window");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        let text = view.composer.input.read(cx).value();
        let mentions = crate::attachment::mentions(&text, &view.composer.attachments);
        assert_eq!(mentions.len(), 2);
        assert_ne!(
            mentions[0].attachment.mention,
            mentions[1].attachment.mention
        );
        assert!(mentions.iter().all(|mention| matches!(
            mention.attachment.content.as_ref(),
            bas::acp::ContentBlock::Resource(_)
        )));
    })
    .expect("window");
}

#[gpui_kit::test]
fn reopening_restores_the_selected_thread_and_deleted_threads_fall_back(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("window state directory");
    let state_path = directory.path().join("window.json");
    let mut previous = session();
    previous.cwd = Some(PathBuf::from("/saved/project"));
    previous.agent = "saved-agent".into();
    let recent = session();
    workspace_navigation::restore::Store::open(state_path.clone())
        .save(Some(previous.id))
        .expect("previous window selection");
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.selected.creating = false;
            view.navigation.restore.store = Some(workspace_navigation::restore::Store::open(
                state_path.clone(),
            ));
            view.navigation.restore.on_connect = true;
            view.update(Update::Catalog(Err("not connected yet".into())), window, cx);
            assert!(
                view.navigation.restore.on_connect,
                "a transient connection failure must not consume restoration"
            );
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: vec![],
                    sessions: vec![recent.clone().into(), previous.clone().into()],
                })),
                window,
                cx,
            );
            assert_eq!(view.selected.id, Some(previous.id));
            assert_eq!(view.navigation.selected_directory, previous.cwd);
            assert_eq!(view.selected.agent, previous.agent.as_str());
        });
    })
    .expect("restore selection");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == previous.id)
    );
    assert!(
        fixture.commands.try_recv().is_err(),
        "restoring must not create a new thread"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.select(recent.id, window, cx));
    })
    .expect("save changed selection");
    assert_eq!(
        workspace_navigation::restore::Store::open(state_path.clone()).thread(),
        Some(recent.id)
    );
    // Simulate a subsequent launch where the saved thread was deleted from bas.
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.selected.id = None;
            view.selected.timeline.conversation = None;
            view.navigation.restore.store = Some(workspace_navigation::restore::Store::open(
                state_path.clone(),
            ));
            view.navigation.restore.on_connect = true;
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: vec![],
                    sessions: vec![previous.clone().into()],
                })),
                window,
                cx,
            );
            assert_eq!(view.selected.id, Some(previous.id));
        });
    })
    .expect("deleted selection fallback");
    assert_eq!(
        workspace_navigation::restore::Store::open(state_path).thread(),
        Some(previous.id)
    );
}
