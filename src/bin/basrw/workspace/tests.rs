use basr::model::{ContentBlock, Cursor, Event, EventKind, Owner};
use gpui_kit::{Focusable, TestAppContext, WindowHandle, size, test::TestWindowExt};
use tokio::sync::mpsc;

use super::*;

mod application;
mod approval;
mod archive;
mod authentication;
mod chrome;
mod composer_expansion;
mod composer_geometry;
mod composer_queue;
mod navigation;
mod rename;
mod rows;
mod settings;
mod thread_actions;
mod timeline;
mod tooltips;

fn receipt(cursor: u64) -> basr::model::TurnReceipt {
    basr::model::TurnReceipt {
        turn: basr::model::TurnId::new(),
        cursor: Cursor::from_raw(cursor),
    }
}

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
        theme::init(None, Some(appearance_path), cx);
        crate::application::init(cx);
        bind_keys(cx);
        crate::settings_window::bind_keys(cx);
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
                "127.0.0.1:7337".parse().expect("test address"),
                PathBuf::from("/tmp"),
                false,
                window,
                cx,
            )
        });
        workspace.update(cx, |view, cx| {
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: vec![pb::Agent {
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
        Root::new(workspace, window, cx)
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

#[gpui_kit::test]
fn title_before_receipt_does_not_drain_and_background_catalog_does(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    let accepted = receipt(3);
    attach(&fixture, first.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .update(cx, |input, cx| input.set_value("A", window, cx));
            view.send_prompt(window, cx);
            view.outboxes
                .get_mut(&first.id)
                .expect("outbox")
                .enqueue(Draft {
                    text: "B".into(),
                    attachments: Vec::new(),
                });
            view.update(
                Update::Event {
                    epoch: view.epoch,
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
                Update::Sent {
                    session: first.id,
                    text: "A".into(),
                    attachments: Vec::new(),
                    result: Ok(accepted),
                },
                window,
                cx,
            );
        });
        assert!(
            matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "A")
        );
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("window");
    attach(&fixture, second.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let mut completed = first.clone();
            completed.revision = Cursor::from_raw(4);
            view.update(Update::Catalog(Ok(connection::Catalog {
                agents: view.agents.clone(), sessions: vec![completed, second],
            })), window, cx);
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session, text, .. }) if session == first.id && text == "B"));
    }).expect("window");
}

#[gpui_kit::test]
fn catalog_and_history_merge_metadata_without_skipping_content(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut newest = session();
    newest.revision = Cursor::from_raw(10);
    newest.title = Some("latest".into());
    newest.status = SessionStatus::Working;
    newest.active_turn = Some(basr::model::TurnId::new());
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
                    turn: basr::model::TurnId::new(),
                    result: Ok(basr::acp::PromptResponse::new(
                        basr::acp::StopReason::EndTurn,
                    )),
                },
            );
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: old.clone(),
                    events: vec![
                        Event::new(
                            newest.id,
                            Cursor::from_raw(4),
                            EventKind::Acp(basr::acp::SessionUpdate::AgentMessageChunk(
                                basr::acp::ContentChunk::new("history".into()),
                            )),
                        ),
                        finish,
                    ],
                },
                window,
                cx,
            );
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.agents.clone(),
                    sessions: vec![old],
                })),
                window,
                cx,
            );
            let conversation = view.conversation.as_ref().expect("conversation");
            assert_eq!(conversation.session, newest);
            assert_eq!(conversation.cursor, 5);
            assert_eq!(conversation.items.len(), 1);
            assert_eq!(view.sessions[0], newest);
            // A completion between history and GetSession stays newer than the snapshot.
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        newest.id,
                        Cursor::from_raw(11),
                        EventKind::TurnFinished {
                            turn: newest.active_turn.expect("turn"),
                            result: Ok(basr::acp::PromptResponse::new(
                                basr::acp::StopReason::EndTurn,
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
                .update(cx, |input, cx| input.set_value("composition", window, cx));
        });
        window.render_frame(cx);
        window.click("pending-messages", cx);
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), "A");
        window.press("escape", cx);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "composition"
        );
        window.render_frame(cx);
        window.click("pending-messages", cx);
        fixture.view.update(cx, |view, cx| {
            view.composer
                .update(cx, |input, cx| input.set_value("A'", window, cx));
            view.send_prompt(window, cx);
            assert_eq!(view.composer.read(cx).value(), "composition");
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::TurnFinished {
                            turn: basr::model::TurnId::new(),
                            result: Ok(basr::acp::PromptResponse::new(
                                basr::acp::StopReason::EndTurn,
                            )),
                        },
                    ),
                },
                window,
                cx,
            );
        });
        assert!(
            matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "A'")
        );
        assert_eq!(
            fixture.view.read(cx).outboxes[&meta.id]
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
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Catalog(Ok(crate::connection::Catalog {
                    agents: view.agents.clone(),
                    sessions: vec![meta.clone()],
                })),
                window,
                cx,
            );
            view.composer
                .update(cx, |input, cx| input.set_value("A", window, cx));
            view.send_prompt(window, cx);
            view.update(
                Update::Catalog(Ok(crate::connection::Catalog {
                    agents: Vec::new(),
                    sessions: Vec::new(),
                })),
                window,
                cx,
            );
            view.update(
                Update::Sent {
                    session: meta.id,
                    text: "A".into(),
                    attachments: Vec::new(),
                    result: Err("late failure".into()),
                },
                window,
                cx,
            );
            assert!(!view.outboxes.contains_key(&meta.id));
            assert!(!view.drafts.contains_key(&meta.id));
        });
    })
    .expect("window");
}

fn attach(fixture: &Fixture, session: SessionMeta, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.push(session.clone());
            view.select(session.id, window, cx);
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session,
                    events: Vec::new(),
                },
                window,
                cx,
            );
            view.stream = StreamState::Live;
            cx.notify();
        });
    })
    .expect("test window available");
}

#[gpui_kit::test]
fn search_shortcut_works_immediately_after_opening(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut session = session();
    session.title = Some("待办任务".into());
    let id = session.id;
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.push(session);
            cx.notify();
        });
        window.render_frame(cx);
        window.press("ctrl-f", cx);
        window.press("ctrl-b", cx);
        window.press("ctrl-f", cx);
        window.input("待办", cx);
        window.press("enter", cx);
    })
    .expect("test window available");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == id)
    );
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
                view.connected = connected;
                view.connection_error = error;
                view.stream = stream;
                view.creating = creating;
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
                .clone()
                .update(cx, |input, cx| input.focus(window, cx));
            window.press("enter", cx);
            assert!(
                fixture.commands.try_recv().is_err(),
                "unready connections must not send"
            );
            assert_eq!(
                fixture.view.read(cx).composer.read(cx).value(),
                "保留这份草稿"
            );
        }
        fixture.view.update(cx, |view, cx| {
            view.connected = true;
            view.connection_error = None;
            view.stream = StreamState::Live;
            view.creating = false;
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
            Ok(Command::Prompt { text, .. }) => assert_eq!(text, "保留这份草稿"),
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
        basr::acp::NewSessionResponse::new(created.id.to_string()).config_options(vec![
            serde_json::from_value(serde_json::json!({
                "id": "thinking", "name": "思考", "type": "boolean", "currentValue": false
            }))
            .expect("ACP boolean option"),
        ]),
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("代码审查", cx);
        window.press("enter", cx);
    })
    .expect("window");
    assert!(
        fixture.commands.try_recv().is_err(),
        "initialization must not send a prompt"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.composer.read(cx).value(), "代码审查");
            let cursor = view.composer.read(cx).cursor_position();
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(created.clone()),
                },
                window,
                cx,
            );
            assert_eq!(view.composer.read(cx).cursor_position(), cursor);
            assert_eq!(view.active, Some(created.id));
            assert_eq!(view.composer.read(cx).value(), "代码审查");
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: created.clone(),
                    events: vec![],
                },
                window,
                cx,
            );
            view.stream = StreamState::Live;
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
                    result: Ok(basr::acp::SetSessionConfigOptionResponse::new(vec![])),
                },
                window,
                cx,
            );
        });
        window.press("enter", cx);
    })
    .expect("window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session, text, .. }) if session == created.id && text == "代码审查")
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
        assert!(fixture.view.read(cx).compact);
        assert!(!fixture.view.read(cx).output_open);
        window.click("toggle-output", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).output_open);
        window.press("ctrl-o", cx);
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).output_open);
        window.press("ctrl-shift-m", cx);
        window.render_frame(cx);
        assert!(window.find("add-workspace-button").visible());
        assert!(window.try_find("toggle-output").is_none());
        assert!(!fixture.view.read(cx).compact);
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.active, Some(session.id));
            assert_eq!(view.composer.read(cx).value(), "保留草稿");
            assert!(view.conversation.is_some());
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
            view.select_agent("other-agent".into(), window, cx);
            assert_eq!(view.composer.read(cx).value(), "保留草稿");
            view.new_thread(Some(PathBuf::from("/other")), window, cx);
        });
        window.input("另一工作区草稿", cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(created.clone()),
                },
                window,
                cx,
            );
            assert_eq!(
                view.selected_directory.as_deref(),
                Some(std::path::Path::new("/other"))
            );
            assert_eq!(view.active, None);
            assert!(view.creating);
            assert_eq!(view.composer.read(cx).value(), "另一工作区草稿");
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
                    result: Ok(latest.clone()),
                },
                window,
                cx,
            );
            assert_eq!(view.active, Some(latest.id));
            assert_eq!(view.composer.read(cx).value(), "另一工作区草稿");
        });
    })
    .expect("window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == latest.id)
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn enter_submits_shift_enter_adds_a_line_and_failed_send_keeps_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::Watch { .. })
    ));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("你好 🦀", cx);
        window.press("shift-enter", cx);
        window.input("第二行", cx);
    })
    .expect("test window available");
    assert!(fixture.commands.try_recv().is_err());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.press("enter", cx)
    })
    .expect("test window available");
    let Command::Prompt {
        session: id, text, ..
    } = fixture.commands.try_recv().expect("prompt sent")
    else {
        panic!("expected Prompt")
    };
    assert_eq!(id, session.id);
    assert_eq!(text, "你好 🦀\n第二行");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Sent {
                    session: id,
                    text: text.clone(),
                    attachments: vec![],
                    result: Err("unavailable".into()),
                },
                window,
                cx,
            );
            assert_eq!(view.composer.read(cx).value().as_ref(), text);
            assert!(!view.sending(id));
            assert!(view.error.is_some());
        });
    })
    .expect("test window available");
}

#[gpui_kit::test]
fn late_events_and_send_receipts_cannot_modify_another_session(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("第一条消息", cx);
        window.press("enter", cx);
    })
    .expect("test window available");
    let old_epoch = cx.update(|cx| fixture.view.read(cx).epoch);
    attach(&fixture, second.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("另一条草稿", cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Sent {
                    session: first.id,
                    text: "第一条消息".into(),
                    attachments: vec![],
                    result: Ok(receipt(1)),
                },
                window,
                cx,
            );
            view.update(
                Update::Event {
                    epoch: old_epoch,
                    event: Event::new(
                        first.id,
                        Cursor::from_raw(99),
                        EventKind::Acp(basr::acp::SessionUpdate::AgentMessageChunk(
                            basr::acp::ContentChunk::new(ContentBlock::from("迟到的回复")),
                        )),
                    ),
                },
                window,
                cx,
            );
            assert_eq!(view.composer.read(cx).value(), "另一条草稿");
            assert!(
                view.conversation
                    .as_ref()
                    .expect("second session")
                    .items
                    .is_empty()
            );
            assert_eq!(view.active, Some(second.id));
            view.select(first.id, window, cx);
            assert!(view.composer.read(cx).value().is_empty());
            view.select(second.id, window, cx);
            assert_eq!(view.composer.read(cx).value(), "另一条草稿");
        });
    })
    .expect("test window available");
    // Drain commands so the test also ensures the fake connection stays alive.
    while fixture.commands.try_recv().is_ok() {}
}

#[gpui_kit::test]
fn archive_row_button_does_not_select(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let _ = fixture.commands.try_recv();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover(format!("session-{}", session.id), cx);
        window.click(format!("archive-{}", session.id), cx);
    })
    .expect("test window available");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Archive { session: id, archived: true }) if id == session.id)
    );
    cx.update(|cx| assert_eq!(fixture.view.read(cx).active, Some(session.id)));
}

#[gpui_kit::test]
fn slash_commands_complete_with_keyboard_and_send_native_prompt(cx: &mut TestAppContext) {
    use basr::acp;
    let mut fixture = fixture(cx);
    let mut session = session();
    session.available_commands = Some(acp::AvailableCommandsUpdate::new(vec![
        acp::AvailableCommand::new("help", "显示帮助"),
        acp::AvailableCommand::new("review", "审查代码").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "文件或目录",
            )),
        ),
    ]));
    attach(&fixture, session.clone(), cx);
    let _ = fixture.commands.try_recv();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("/", cx);
        window.render_frame(cx);
        assert!(window.find("slash-review").visible());
        window.press("down", cx);
        window.press("tab", cx);
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), "/review ");
        assert_eq!(
            fixture.view.read(cx).command_hint(cx).as_deref(),
            Some("文件或目录")
        );
        window.input("src/", cx);
        window.press("enter", cx);
    })
    .expect("window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session: id, text, .. }) if id == session.id && text == "/review src/")
    );
    assert!(fixture.commands.try_recv().is_err());
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
            assert!(!view.creating);
            assert_eq!(view.composer.read(cx).value(), "尚未发送");
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
fn streamed_markdown_keeps_its_entity_and_renders_user_formatting(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for (cursor, text) in [(1, "**粗"), (2, "体** 与 [链接](https://example.com)")] {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(
                            session.id,
                            Cursor::from_raw(cursor),
                            EventKind::Acp(basr::acp::SessionUpdate::UserMessageChunk(
                                basr::acp::ContentChunk::new(text.into()),
                            )),
                        ),
                    },
                    window,
                    cx,
                );
            }
            let first = view.markdown("probe".into(), "**粗".into(), 0, cx);
            drop(first);
            let entity = view.markdown.borrow()["probe"].state.entity_id();
            drop(view.markdown(
                "probe".into(),
                "**粗体** 与 [链接](https://example.com)".into(),
                0,
                cx,
            ));
            assert_eq!(view.markdown.borrow()["probe"].state.entity_id(), entity);
        });
        window.render_frame(cx);
        window.press("ctrl-shift-m", cx);
        window.press("ctrl-o", cx);
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        let id = view.conversation.as_ref().expect("conversation").items[0].id();
        assert!(
            window.find(SharedString::from(id)).visible(),
            "compact mode also shows user messages"
        );
    })
    .expect("window");
    cx.run_until_parked();
    cx.update(|cx| {
        let document = fixture.view.read(cx).markdown.borrow()["probe"]
            .state
            .clone();
        document.update(cx, |state, cx| {
            state.select_all(cx);
            assert_eq!(state.selected_text().trim(), "粗体 与 链接");
        });
    });
}

#[gpui_kit::test]
fn command_menu_handles_escape_mouse_selection_and_live_catalog_replacement(
    cx: &mut TestAppContext,
) {
    use basr::acp;
    let mut fixture = fixture(cx);
    let mut session = session();
    session.available_commands = Some(acp::AvailableCommandsUpdate::new(vec![
        acp::AvailableCommand::new("help", "帮助"),
    ]));
    attach(&fixture, session.clone(), cx);
    let _ = fixture.commands.try_recv();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("/", cx);
    })
    .expect("window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
        assert!(
            !fixture.view.read(cx).compact,
            "Escape only closes the menu"
        );
        assert!(fixture.view.read(cx).command_candidates(cx).is_empty());
        window.input("h", cx);
    })
    .expect("window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("slash-help", cx);
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), "/help ");
        fixture.view.update(cx, |view, cx| {
            view.composer
                .update(cx, |input, cx| input.set_value("/", window, cx));
            view.commands_dismissed = false;
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(acp::SessionUpdate::AvailableCommandsUpdate(
                            acp::AvailableCommandsUpdate::new(vec![acp::AvailableCommand::new(
                                "review", "审查",
                            )]),
                        )),
                    ),
                },
                window,
                cx,
            );
            assert_eq!(
                view.command_candidates(cx)
                    .iter()
                    .map(|command| command.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["review"]
            );
            view.composer.update(cx, |input, cx| {
                input.set_value("/unknown raw arguments", window, cx)
            });
        });
        window.press("enter", cx);
    })
    .expect("window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "/unknown raw arguments")
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn compact_menus_escape_window_bounds_and_keep_the_composer_small(cx: &mut TestAppContext) {
    use basr::acp;
    let mut fixture = fixture(cx);
    let mut session = session();
    session.acp = Some(
        acp::NewSessionResponse::new(session.id.to_string()).config_options(vec![
            acp::SessionConfigOption::select(
                "model",
                "Model",
                "0",
                (0..24)
                    .map(|i| {
                        acp::SessionConfigSelectOption::new(i.to_string(), format!("Model {i}"))
                    })
                    .collect::<Vec<_>>(),
            ),
        ]),
    );
    attach(&fixture, session, cx);
    while fixture.commands.try_recv().is_ok() {}
    let (popup, compact_size) = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.press("ctrl-shift-m", cx);
            window.bounds_changed(cx);
            window.render_frame(cx);
            let before = window.viewport_size();
            window.click("config-model", cx);
            let popup = fixture
                .view
                .read(cx)
                .menu
                .as_ref()
                .expect("open menu")
                .window
                .expect("native popup");
            assert_eq!(before, window.viewport_size());
            assert!(fixture.view.read(cx).compact);
            (popup, before)
        })
        .expect("parent window");
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.find("popup-menu").bounds();
        assert!(
            menu.size.height > compact_size.height,
            "menu is taller than the parent window"
        );
        assert!(menu.bottom() <= window.viewport_size().height);
        for _ in 0..24 {
            window.press("down", cx);
        }
        window.render_frame(cx);
        let last = window.within("popup-menu").find(23usize).bounds();
        assert!(
            last.top() >= px(0.) && last.bottom() <= window.viewport_size().height,
            "last menu item must scroll into view: {last:?}"
        );
        window.press("enter", cx);
    })
    .expect("menu window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.viewport_size(), compact_size);
        assert!(fixture.view.read(cx).menu.is_none());
        assert!(fixture.view.read(cx).compact);
        assert!(!cx.windows().contains(&popup));
        window.click("workspace-picker", cx);
    })
    .expect("parent window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::SetConfig(request))
        if request.value == acp::SessionConfigOptionValue::value_id("23"))
    );
    let popup = cx.update(|cx| {
        fixture
            .view
            .read(cx)
            .menu
            .as_ref()
            .expect("workspace menu")
            .window
            .expect("native popup")
    });
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .expect("menu window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(fixture.view.read(cx).menu.is_none());
        assert!(
            fixture.view.read(cx).compact,
            "Escape should only close the menu"
        );
        assert_eq!(window.viewport_size(), compact_size);
        window.click("agent-picker", cx);
        assert!(fixture.view.read(cx).menu.is_some());
        window.click("composer-editor", cx);
        assert!(
            fixture.view.read(cx).menu.is_none(),
            "clicking the parent dismisses the menu"
        );
    })
    .expect("parent window");
}

#[gpui_kit::test]
fn compact_menu_actions_use_the_parent_window(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let popup = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.agents.push(pb::Agent {
                    id: "other".into(),
                    kind: "acp".into(),
                    command: String::new(),
                });
                cx.notify();
            });
            window.press("ctrl-shift-m", cx);
            window.bounds_changed(cx);
            window.render_frame(cx);
            window.input("保留草稿", cx);
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
        .expect("parent window");
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .expect("agent popup");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Detach)));
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Create(request)) if request.agent == "other")
    );
    let (popup, add_index) = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert!(view.menu.is_none());
            assert_eq!(view.composer.read(cx).value(), "保留草稿");
            let add_index = groups::group_threads(&view.sessions, &view.directories, "").len() + 1;
            window.click("workspace-picker", cx);
            (
                fixture
                    .view
                    .read(cx)
                    .menu
                    .as_ref()
                    .expect("workspace menu")
                    .window
                    .expect("popup"),
                add_index,
            )
        })
        .expect("parent window");
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(add_index, cx);
    })
    .expect("workspace popup");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert!(view.menu.is_none());
        assert!(!view.compact);
        assert!(view.editing_workspace);
        assert!(view.cwd.read(cx).focus_handle(cx).is_focused(window));
        assert!(!cx.windows().contains(&popup));
    })
    .expect("parent window");
}

#[gpui_kit::test]
fn composer_toolbar_keeps_options_attachments_and_send_on_one_row_in_both_modes(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let mut session = session();
    session.acp = Some(
        basr::acp::NewSessionResponse::new(session.id.to_string()).config_options(vec![
            basr::acp::SessionConfigOption::boolean("thinking", "思考", true),
        ]),
    );
    attach(&fixture, session, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        for compact in [false, true] {
            if compact {
                window.press("ctrl-shift-m", cx);
            }
            window.render_frame(cx);
            let toolbar = window.find("composer-toolbar").bounds();
            let send = window.find("send-message").bounds();
            assert_eq!(
                fixture
                    .view
                    .read(cx)
                    .composer
                    .read(cx)
                    .cursor_layout()
                    .expect("caret layout")
                    .0
                    .left(),
                window.find("add-attachment").bounds().left(),
                "the text caret and first toolbar button share the left edge"
            );
            for id in ["add-attachment", "config-thinking", "agent-picker"] {
                let control = window.find(id);
                assert!(control.visible());
                assert_eq!(
                    control.bounds().origin.y,
                    send.origin.y,
                    "{id} must align with send"
                );
                assert!(control.bounds().origin.y >= toolbar.origin.y);
            }
            assert!(send.origin.x + send.size.width <= toolbar.origin.x + toolbar.size.width);
        }
    })
    .expect("window");
}

#[gpui_kit::test]
fn explicit_line_break_at_end_increases_the_editor_height(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("line", cx);
        window.render_frame(cx);
        let initial_height = window.find("composer-editor").bounds().size.height;
        window.press("shift-enter", cx);
        window.render_frame(cx);

        assert!(
            fixture
                .view
                .read(cx)
                .composer
                .read(cx)
                .value()
                .ends_with('\n')
        );
        assert!(
            window.find("composer-editor").bounds().size.height > initial_height,
            "the trailing empty line must contribute to the editor height"
        );
    })
    .expect("composer window");
}

#[gpui_kit::test]
fn soft_wrapped_composer_text_increases_the_editor_height(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let initial_height = window.find("composer-editor").bounds().size.height;
        window.input(&"long text without an explicit line break ".repeat(12), cx);
        window.render_frame(cx);
        window.render_frame(cx);

        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value().lines().count(),
            1,
            "this regression exercises soft wrapping, not explicit newlines"
        );
        let wrapped_height = window.find("composer-editor").bounds().size.height;
        assert!(
            wrapped_height > initial_height,
            "soft-wrapped text must grow the composer: initial={initial_height:?}, wrapped={wrapped_height:?}, rows={}, text_bounds={:?}",
            fixture.view.read(cx).composer_rows,
            fixture.view.read(cx).composer.read(cx).text_bounds(),
        );
    })
    .expect("composer window");
}

#[gpui_kit::test]
fn attachments_follow_drafts_and_survive_failed_send(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut first = session();
    first.initialization =
        Some(
            basr::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
                .agent_capabilities(basr::acp::AgentCapabilities::new().prompt_capabilities(
                    basr::acp::PromptCapabilities::new().embedded_context(true),
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
            view.sessions.push(second.clone());
            view.select(second.id, window, cx);
            assert!(view.attachments.is_empty());
            view.select(first.id, window, cx);
            assert_eq!(view.attachments[0].id, attachment.id);
            view.stream = StreamState::Live;
            view.send_prompt(window, cx);
            view.update(
                Update::Sent {
                    session: first.id,
                    text: format!("{} ", attachment.mention),
                    attachments: vec![attachment.clone()],
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            assert_eq!(view.attachments[0].id, attachment.id);
            view.send_prompt(window, cx);
            view.update(
                Update::Sent {
                    session: first.id,
                    text: format!("{} ", attachment.mention),
                    attachments: vec![attachment.clone()],
                    result: Ok(receipt(1)),
                },
                window,
                cx,
            );
            assert!(view.attachments.is_empty());
        });
    })
    .expect("window");
    let mut sent = false;
    while let Ok(command) = fixture.commands.try_recv() {
        if let Command::Prompt {
            text, attachments, ..
        } = command
        {
            assert_eq!(text, format!("{} ", attachment.mention));
            assert_eq!(attachments[0].id, attachment.id);
            sent = true;
        }
    }
    assert!(sent);
}

#[gpui_kit::test]
fn long_tool_titles_leave_room_for_status_in_both_modes(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let tool = basr::acp::ToolCall::new("long-command", "echo '检查环境'; ".repeat(50))
                .status(basr::acp::ToolCallStatus::Completed);
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCall(tool)),
                    ),
                },
                window,
                cx,
            );
        });
        for compact in [false, true] {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.output_open = true;
                view.resize_window(window, cx);
            });
            window.bounds_changed(cx);
            window.render_frame(cx);
            let card = window.find("tool-long-command").bounds();
            let title = window.find("expand-tool-long-command").bounds();
            let status = window.find("status-tool-long-command").bounds();
            assert!(
                status.right() <= card.right(),
                "tool status is outside its card"
            );
            assert!(title.right() <= status.left(), "title overlaps tool status");
            assert!(
                status.size.width >= px(16.),
                "tool status must not shrink away"
            );
            window.click("expand-tool-long-command", cx);
            assert!(fixture.view.read(cx).expanded.contains("tool-long-command"));
            window.click("expand-tool-long-command", cx);
        }
    })
    .expect("window");
}

#[gpui_kit::test]
fn terminal_reference_does_not_hide_raw_tool_result(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let tool = basr::acp::ToolCall::new("shell", "fastfetch")
                .content(vec![basr::acp::ToolCallContent::Terminal(
                    basr::acp::Terminal::new("remote"),
                )])
                .raw_output(serde_json::json!("Linux 6.x\n结果"));
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCall(tool)),
                    ),
                },
                window,
                cx,
            );
            view.expanded.insert("tool-shell".into());
        });
        window.render_frame(cx);
        assert!(
            fixture
                .view
                .read(cx)
                .markdown
                .borrow()
                .contains_key("tool-shell-output"),
            "raw output must remain visible when a terminal reference has no stream"
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn compact_output_precedes_input_and_full_status_shares_content_width(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.resize(size(px(1800.), px(1000.)));
        window.render_frame(cx);
        let composer = window.find("composer-panel").bounds();
        let status = window.find("conversation-status").bounds();
        assert_eq!(composer.origin.x, status.origin.x);
        assert_eq!(composer.size.width, status.size.width);
        let scroll = window.find("scroll-to-bottom").bounds();
        assert!(scroll.right() <= composer.right());
        window.press("ctrl-shift-m", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        let editor = window.find("composer-editor").bounds();
        assert!(
            editor.origin.y < px(40.),
            "first row should follow the compact status bar"
        );
        assert!(window.find("compact-status").visible());
        assert!(window.try_find("agent-output").is_none());
        let bottom_gap =
            window.viewport_size().height - window.find("composer-toolbar").bounds().bottom();
        assert!(
            bottom_gap <= px(5.),
            "unused space below toolbar: {bottom_gap:?}"
        );
        let composer = fixture.view.read(cx).composer.clone();
        composer.update(cx, |input, cx| {
            input.set_scroll_offset(gpui_kit::point(px(0.), px(-1000.)), cx);
        });
        window.render_frame(cx);
        assert_eq!(
            composer.read(cx).scroll_offset().y,
            px(0.),
            "empty composer must not scroll"
        );
        window.press("ctrl-o", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        let output = window.find("agent-output").bounds();
        assert!(output.bottom() <= window.find("composer-editor").bounds().top());
        assert!(output.size.height > px(100.));
    })
    .expect("window");
}

#[gpui_kit::test]
fn clipboard_images_are_inline_and_support_delete_undo_and_text_paste(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut session = session();
    session.initialization = Some(
        basr::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .agent_capabilities(
                basr::acp::AgentCapabilities::new()
                    .prompt_capabilities(basr::acp::PromptCapabilities::new().image(true)),
            ),
    );
    attach(&fixture, session, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("前后", cx);
        let composer = fixture.view.read(cx).composer.clone();
        composer.update(cx, |input, cx| input.set_selected_range(3..3, cx));
        let image =
            gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b"\x89PNG\r\n\x1a\n".to_vec());
        cx.write_to_clipboard(ClipboardItem::new_image(&image));
        window.press("ctrl-v", cx);
        window.render_frame(cx);
        let value = fixture.view.read(cx).composer.read(cx).value();
        let attachment = fixture.view.read(cx).attachments[0].clone();
        assert_eq!(value, format!("前{} 后", attachment.mention));
        assert_eq!(attachment.label, "Image");
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
            &view.attachments,
            &view.prompt_capabilities(),
        );
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[1], basr::acp::ContentBlock::Image(_)));
        let end = "前".len() + attachment.mention.len();
        composer.update(cx, |input, cx| input.set_selected_range(end..end, cx));
        window.press("backspace", cx);
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), "前 后");
        window.press("ctrl-z", cx);
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), value);
        cx.write_to_clipboard(ClipboardItem::new_string("普通文本".into()));
        window.press("ctrl-v", cx);
        assert!(
            fixture
                .view
                .read(cx)
                .composer
                .read(cx)
                .value()
                .contains("普通文本")
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn word_deletion_removes_a_dragged_image_atomically_and_undo_restores_it(cx: &mut TestAppContext) {
    use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};

    let fixture = fixture(cx);
    let mut session = session();
    session.initialization = Some(
        basr::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .agent_capabilities(
                basr::acp::AgentCapabilities::new()
                    .prompt_capabilities(basr::acp::PromptCapabilities::new().image(true)),
            ),
    );
    attach(&fixture, session, cx);
    let directory = tempfile::tempdir().expect("image fixture directory");
    let path = directory.path().join("image.png");
    std::fs::write(&path, b"\x89PNG\r\n\x1a\n").expect("write image fixture");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("composer-panel").bounds().center();
        window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Entered {
                position,
                paths: ExternalPaths([path].into_iter().collect()),
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::FileDrop(FileDropEvent::Submit { position }),
            cx,
        );
    })
    .expect("drop image into composer");
    cx.executor().run_until_parked();
    let original = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let original = fixture.view.read(cx).composer.read(cx).value();
            assert_eq!(
                crate::attachment::mentions(&original, &fixture.view.read(cx).attachments).len(),
                1
            );
            window.press("ctrl-backspace", cx);
            original
        })
        .expect("delete image");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let composer = fixture.view.read(cx).composer.clone();
        assert_eq!(
            composer.read(cx).value(),
            "",
            "word deletion must remove the whole attachment marker, not its closing bracket"
        );
        window.press("ctrl-z", cx);
        assert_eq!(composer.read(cx).value(), original);
        assert_eq!(
            crate::attachment::mentions(
                &composer.read(cx).value(),
                &fixture.view.read(cx).attachments
            )
            .len(),
            1,
            "undo must restore the original attachment reference"
        );
    })
    .expect("delete and restore image");
}

#[gpui_kit::test]
fn word_deletion_preserves_attachment_boundaries_and_undo_carets(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    let image =
        gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b"\x89PNG\r\n\x1a\n".to_vec());
    let attachment = Attachment::image(&image).expect("test image attachment");
    let prefix = "前文😀 ";
    let suffix = " 后文";
    for backward in [true, false] {
        for padding in ["", " ", "\t"] {
            let value = if backward {
                format!("{prefix}{}{padding}{suffix}", attachment.mention)
            } else {
                format!("{prefix}{padding}{}{suffix}", attachment.mention)
            };
            let caret = if backward {
                prefix.len() + attachment.mention.len() + padding.len()
            } else {
                prefix.len()
            };
            cx.update_window(fixture.window.into(), |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.attachments = vec![attachment.clone()];
                    view.composer.update(cx, |input, cx| {
                        input.set_value(value.clone(), window, cx);
                        input.set_selected_range(caret..caret, cx);
                        input.focus(window, cx);
                    });
                    view.sync_mentions(cx);
                });
                window.render_frame(cx);
                window.press(
                    if backward {
                        "ctrl-backspace"
                    } else {
                        "ctrl-delete"
                    },
                    cx,
                );
                let composer = fixture.view.read(cx).composer.clone();
                assert_eq!(
                    composer.read(cx).value(),
                    format!("{prefix}{suffix}"),
                    "backward={backward}, padding={padding:?}"
                );
                window.press("ctrl-z", cx);
                assert_eq!(composer.read(cx).value(), value);
                assert_eq!(
                    composer.read(cx).selected_range(),
                    caret..caret,
                    "undo must restore the original caret, not a temporary selection"
                );
            })
            .expect("delete and restore adjacent attachment");
        }
    }
}

#[gpui_kit::test]
fn word_deletion_keeps_native_text_and_existing_selection_semantics(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    let image =
        gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b"\x89PNG\r\n\x1a\n".to_vec());
    let attachment = Attachment::image(&image).expect("test image attachment");
    for has_attachment in [false, true] {
        for backward in [true, false] {
            let tail = if has_attachment {
                format!(" {}", attachment.mention)
            } else {
                String::new()
            };
            let value = format!("one two{tail}");
            let caret = if backward { "one two".len() } else { 0 };
            cx.update_window(fixture.window.into(), |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.attachments = if has_attachment {
                        vec![attachment.clone()]
                    } else {
                        Vec::new()
                    };
                    view.composer.update(cx, |input, cx| {
                        input.set_value(value.clone(), window, cx);
                        input.set_selected_range(caret..caret, cx);
                        input.focus(window, cx);
                    });
                    view.sync_mentions(cx);
                });
                window.render_frame(cx);
                window.press(
                    if backward {
                        "ctrl-backspace"
                    } else {
                        "ctrl-delete"
                    },
                    cx,
                );
                let expected = if backward {
                    format!("one {tail}")
                } else {
                    format!(" two{tail}")
                };
                let composer = fixture.view.read(cx).composer.clone();
                assert_eq!(composer.read(cx).value(), expected);
                window.press("ctrl-z", cx);
                assert_eq!(composer.read(cx).value(), value);
                assert_eq!(composer.read(cx).selected_range(), caret..caret);
            })
            .expect("ordinary word deletion retains native undo");
        }
    }

    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.attachments = vec![attachment.clone()];
            view.composer.update(cx, |input, cx| {
                input.set_value(format!("keep {} after", attachment.mention), window, cx);
                input.set_selected_range(5..5 + attachment.mention.len(), cx);
            });
            view.sync_mentions(cx);
        });
        window.render_frame(cx);
        window.press("ctrl-backspace", cx);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "keep  after",
            "an existing selection must not expand to the previous word"
        );
    })
    .expect("delete selected attachment");
}

#[gpui_kit::test]
fn compact_status_tracks_thought_tools_and_answer_without_expanding(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.press("ctrl-shift-m", cx);
        window.render_frame(cx);
        window.input("开始", cx);
        window.press("enter", cx);
        assert!(!fixture.view.read(cx).output_open);
        fixture.view.update(cx, |view, cx| {
            view.outboxes.clear();
            let conversation = view.conversation.as_mut().expect("conversation");
            conversation.session.status = SessionStatus::Working;
            for (index, (update, status)) in [
                (
                    basr::acp::SessionUpdate::AgentThoughtChunk(basr::acp::ContentChunk::new(
                        "想一想".into(),
                    )),
                    "思考中…",
                ),
                (
                    basr::acp::SessionUpdate::ToolCall(
                        basr::acp::ToolCall::new("shell", "检查文件")
                            .status(basr::acp::ToolCallStatus::InProgress),
                    ),
                    "调用工具 · 检查文件",
                ),
                (
                    basr::acp::SessionUpdate::AgentMessageChunk(basr::acp::ContentChunk::new(
                        "结果".into(),
                    )),
                    "正在输出…",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(
                            session.id,
                            Cursor::from_raw(index as u64 + 1),
                            EventKind::Acp(update),
                        ),
                    },
                    window,
                    cx,
                );
                assert_eq!(view.stream_label(), status);
                assert!(!view.output_open);
            }
        });
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
            assert!(view.attachments.is_empty());
            assert_eq!(view.composer.read(cx).value(), "原有草稿");
            assert!(
                view.send_hint(cx)
                    .starts_with("当前 Agent 不支持此附件类型")
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
        assert!(fixture.view.read(cx).error.is_none());
        assert!(
            fixture
                .view
                .read(cx)
                .send_hint(cx)
                .starts_with("发送 · Enter")
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
            assert_eq!(view.attachments.len(), 1);
            assert!(view.can_send(cx));
            assert!(view.error.is_none());
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
            view.search.update(cx, |input, cx| input.focus(window, cx));
            view.add_attachments(
                session.id,
                vec![Attachment::image(&image).expect("PNG")],
                window,
                cx,
            );
            assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
            assert!(view.attachments.is_empty());
            assert_eq!(view.composer.read(cx).value(), "草稿");
            assert!(view.can_send(cx));
        });
        window.press("escape", cx);
        assert!(fixture.view.read(cx).error.is_none());
        window.press("backspace", cx);
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), "草");
        window.press("enter", cx);
    })
    .expect("window");
    let mut sent = false;
    // The foreground update may accompany a render or activation.
    while let Ok(command) = fixture.commands.try_recv() {
        if let Command::Prompt {
            text, attachments, ..
        } = command
        {
            assert_eq!(text, "草");
            assert!(attachments.is_empty());
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
            view.compact = true;
            view.output_open = false;
            view.resize_window(window, cx);
            for event in &events {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: event.clone(),
                    },
                    window,
                    cx,
                );
            }
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).seen_replies.contains_key(&session.id));
        assert!(window.find("unread-reply").bounds().size.width > px(0.));
        let trigger = window.find("completed-reply").bounds();
        fixture.view.update(cx, |view, cx| {
            view.open_reply_preview(trigger, window, cx);
            assert!(view.reply_preview.is_some());
            assert!(
                !view.seen_replies.contains_key(&session.id),
                "preview must not consume unread status"
            );
        });
        assert!(window.is_window_active(), "preview must not take focus");
        window.click("completed-reply", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).output_open);
        assert!(fixture.view.read(cx).reply_preview.is_none());
        assert_eq!(
            fixture.view.read(cx).seen_replies.get(&session.id),
            Some(&2)
        );
        fixture.view.update(cx, |view, cx| {
            view.toggle_output(window, cx);
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: session.clone(),
                    events,
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(
            fixture.view.read(cx).seen_replies.get(&session.id),
            Some(&2)
        );
        assert!(!fixture.view.read(cx).output_open);
    })
    .expect("reply window");
}

fn completed_events(session: SessionId, text: &str) -> Vec<Event> {
    let turn = basr::model::TurnId::new();
    [
        EventKind::TurnStarted { turn },
        EventKind::Acp(basr::acp::SessionUpdate::AgentMessageChunk(
            basr::acp::ContentChunk::new(text.into()),
        )),
        EventKind::TurnFinished {
            turn,
            result: Ok(basr::acp::PromptResponse::new(
                basr::model::StopReason::EndTurn,
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
                        epoch: view.epoch,
                        event,
                    },
                    window,
                    cx,
                );
            }
            view.list.set_follow_mode(FollowMode::Normal);
            view.list.scroll_to(gpui_kit::ListOffset::default());
        });
        window.render_frame(cx);
        assert!(!primary.view.read(cx).seen_replies.contains_key(&first.id));
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
            !primary.view.read(cx).seen_replies.contains_key(&first.id),
            "the final answer is below the viewport"
        );
        window.click("scroll-to-bottom", cx);
        window.render_frame(cx);
        assert_eq!(primary.view.read(cx).seen_replies.get(&first.id), Some(&2));
    })
    .expect("read latest answer");
    let second = session();
    attach(&primary, second.clone(), cx);
    cx.update_window(primary.window.into(), |_, window, cx| {
        primary.view.update(cx, |view, cx| {
            view.compact = true;
            view.output_open = false;
            for event in completed_events(second.id, "另一个会话的新回复") {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event,
                    },
                    window,
                    cx,
                );
            }
        });
        window.render_frame(cx);
        assert!(!primary.view.read(cx).seen_replies.contains_key(&second.id));
        assert_eq!(primary.view.read(cx).seen_replies.get(&first.id), Some(&2));
    })
    .expect("separate unread state");
}

#[gpui_kit::test]
fn running_turn_queues_messages_in_order_and_keeps_failed_messages(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut session = session();
    let accepted = receipt(2);
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
        assert!(fixture.commands.try_recv().is_err(), "running turns must not overlap");
        assert_eq!(fixture.view.read(cx).outboxes.get(&session.id).map(|q| q.len()), Some(2), "text after enter: {:?}", fixture.view.read(cx).composer.read(cx).value());
        assert!(fixture.view.read(cx).composer.read(cx).value().is_empty());
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Event { epoch: view.epoch, event: Event::new(session.id, Cursor::from_raw(1), EventKind::TurnFinished {
                turn: basr::model::TurnId::new(), result: Ok(basr::acp::PromptResponse::new(basr::acp::StopReason::EndTurn))
            }) }, window, cx);
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "追加一"));
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Sent { session: session.id, text: "追加一".into(), attachments: Vec::new(), result: Ok(accepted) }, window, cx);
        });
        assert!(fixture.commands.try_recv().is_err(), "receipt must not drain the next prompt using a stale Idle snapshot");
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Event { epoch: view.epoch, event: Event::new(session.id, Cursor::from_raw(2), EventKind::TurnStarted { turn: accepted.turn }) }, window, cx);
            view.update(Update::Event { epoch: view.epoch, event: Event::new(session.id, Cursor::from_raw(3), EventKind::TurnFinished {
                turn: accepted.turn, result: Ok(basr::acp::PromptResponse::new(basr::acp::StopReason::EndTurn))
            }) }, window, cx);
        });
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "追加二"));
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Sent { session: session.id, text: "追加二".into(), attachments: Vec::new(), result: Err("offline".into()) }, window, cx);
            view.flush_queue(cx);
        });
        assert!(fixture.commands.try_recv().is_err(), "failed queued sends must wait for user retry");
        window.render_frame(cx);
        window.click("pending-messages", cx);
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), "追加二");
    }).expect("window");
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
            crate::attachment::mentions(&view.composer.read(cx).value(), &view.attachments).len(),
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
        let text = view.composer.read(cx).value();
        let mentions = crate::attachment::mentions(&text, &view.attachments);
        assert_eq!(mentions.len(), 2);
        assert_ne!(
            mentions[0].attachment.mention,
            mentions[1].attachment.mention
        );
        assert!(mentions.iter().all(|mention| matches!(
            mention.attachment.content.as_ref(),
            basr::acp::ContentBlock::Resource(_)
        )));
    })
    .expect("window");
}

#[gpui_kit::test]
fn multiline_commands_keep_a_single_line_header_and_original_expanded_content(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let command = "bash -lc 'echo first\n  printf \"second\\n\"\n  echo 中文'";
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCall(
                            basr::acp::ToolCall::new("multi", command)
                                .status(basr::acp::ToolCallStatus::Completed),
                        )),
                    ),
                },
                window,
                cx,
            );
        });
        for compact in [false, true] {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.output_open = true;
                view.resize_window(window, cx);
            });
            window.bounds_changed(cx);
            window.render_frame(cx);
            assert!(window.find("expand-tool-multi").bounds().size.height < px(36.));
            let status = window.find("status-tool-multi").bounds();
            assert!(status.right() <= window.find("tool-multi").bounds().right());
            window.click("expand-tool-multi", cx);
            window.render_frame(cx);
            assert!(
                fixture
                    .view
                    .read(cx)
                    .markdown
                    .borrow()
                    .contains_key("tool-multi-command")
            );
            window.click("expand-tool-multi", cx);
        }
    })
    .expect("window");
}

#[gpui_kit::test]
fn attachment_chip_wraps_as_one_unit_and_arrows_cross_it_atomically(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let directory = tempfile::tempdir().expect("attachment directory");
    let path = directory.path().join("keys.nix");
    std::fs::write(&path, "{ enabled = true; }").expect("attachment file");
    let attachment = Attachment::load(&path).expect("text attachment");
    for length in [31, 37, 39] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.compact = true;
                view.attachments.clear();
                view.composer.update(cx, |input, cx| {
                    input.set_value("前".repeat(length), window, cx);
                    let end = input.value().len();
                    input.set_selected_range(end..end, cx);
                });
                view.add_attachments(session.id, vec![attachment.clone()], window, cx);
            });
            window.bounds_changed(cx);
            window.render_frame(cx);
        })
        .expect("insert attachment near wrap boundary");
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            let editor = view.composer.clone();
            let range = length * 3..length * 3 + attachment.mention.len();
            let bounds = editor
                .read(cx)
                .range_to_bounds(&range)
                .expect("attachment layout");
            assert_eq!(
                bounds.size.height,
                editor.read(cx).line_height().expect("line height"),
                "attachment must occupy one row"
            );
            assert!(bounds.size.width > px(35.));
            assert!(
                window
                    .find("composer-editor")
                    .bounds()
                    .contains(&bounds.center())
            );
            editor.update(cx, |input, cx| {
                input.set_selected_range(range.start..range.start, cx)
            });
            window.press("right", cx);
        })
        .expect("cross attachment");
        cx.update_window(fixture.window.into(), |_, _, cx| {
            let editor = fixture.view.read(cx).composer.read(cx);
            assert_eq!(
                editor.selected_range(),
                (length * 3 + attachment.mention.len())..(length * 3 + attachment.mention.len())
            );
        })
        .expect("atomic arrow navigation");
    }
}

#[gpui_kit::test]
fn reopening_restores_the_selected_thread_and_deleted_threads_fall_back(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("window state directory");
    let state_path = directory.path().join("window.json");
    let mut previous = session();
    previous.cwd = Some(PathBuf::from("/saved/project"));
    previous.agent = "saved-agent".into();
    let recent = session();
    restore::Store::open(state_path.clone())
        .save(Some(previous.id))
        .expect("previous window selection");
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.creating = false;
            view.restore_store = Some(restore::Store::open(state_path.clone()));
            view.restore_on_connect = true;
            view.update(Update::Catalog(Err("not connected yet".into())), window, cx);
            assert!(
                view.restore_on_connect,
                "a transient connection failure must not consume restoration"
            );
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: vec![],
                    sessions: vec![recent.clone(), previous.clone()],
                })),
                window,
                cx,
            );
            assert_eq!(view.active, Some(previous.id));
            assert_eq!(view.selected_directory, previous.cwd);
            assert_eq!(view.selected_agent, previous.agent.as_str());
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
        restore::Store::open(state_path.clone()).thread(),
        Some(recent.id)
    );
    // Simulate a subsequent launch where the saved thread was deleted from basrd.
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.active = None;
            view.conversation = None;
            view.restore_store = Some(restore::Store::open(state_path.clone()));
            view.restore_on_connect = true;
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: vec![],
                    sessions: vec![previous.clone()],
                })),
                window,
                cx,
            );
            assert_eq!(view.active, Some(previous.id));
        });
    })
    .expect("deleted selection fallback");
    assert_eq!(restore::Store::open(state_path).thread(), Some(previous.id));
}
