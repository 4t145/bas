use super::*;
use bas::{acp, model::SessionSnapshot};

struct HistoryWindow {
    window: WindowHandle<Root>,
    commands: mpsc::UnboundedReceiver<Command>,
    updates: mpsc::Sender<Update>,
}

fn open_history(fixture: &Fixture, cx: &mut TestAppContext) -> HistoryWindow {
    let (commands, receiver) = mpsc::unbounded_channel();
    let (updates, events) = mpsc::channel(64);
    let request = cx.update(|cx| {
        fixture.view.update(cx, |view, cx| {
            let mut request = view.history_request("history-agent".into(), cx);
            request.agents.push(connection::AgentSummary {
                id: "history-agent".into(),
                kind: "acp".into(),
                command: String::new(),
            });
            request
        })
    });
    let window = cx.update(|cx| {
        crate::history::window::open_connected(request, Connection::for_test(commands), events, cx)
            .expect("independent history window")
    });
    HistoryWindow {
        window,
        commands: receiver,
        updates,
    }
}

fn listing() -> connection::AgentHistory {
    connection::AgentHistory {
        sessions: acp::ListSessionsResponse::new(vec![
            acp::SessionInfo::new("external/id", "/tmp").title("Native CLI history"),
        ]),
        initialization: acp::InitializeResponse::new(
            agent_client_protocol::schema::ProtocolVersion::V1,
        )
        .agent_capabilities(acp::AgentCapabilities::new().load_session(true)),
        imported: HashMap::new(),
    }
}

#[gpui_kit::test]
fn reopening_restarted_current_session_reattaches_without_clearing_the_draft(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Stopped;
    attach(&fixture, meta.clone(), cx);
    let callback = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.selected.stream = StreamState::Closed;
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("keep unsent text", window, cx)
                });
                view.history_request(meta.agent.to_string(), cx)
                    .open_session
            })
        })
        .expect("closed stream");
    while fixture.commands.try_recv().is_ok() {}
    meta.status = SessionStatus::Idle;
    meta.revision = Cursor::from_raw(10);
    cx.update(|cx| assert!(callback(meta.clone().into(), cx)));
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == meta.id)
    );
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(meta.id));
        assert_eq!(view.selected.stream, StreamState::Loading);
        assert_eq!(view.composer.input.read(cx).value(), "keep unsent text");
    });
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn workspace_restart_retries_failure_and_preserves_the_current_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.agent = "unsupported-acp".into();
    meta.status = SessionStatus::Stopped;
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.request_restart_agent_session(meta.id, cx);
            assert!(fixture.commands.try_recv().is_err(), "unsupported Agent cannot restart");
            meta.initialization = Some(listing().initialization);
            *view.sessions.entries.iter_mut().find(|entry| entry.id == meta.id).expect("session") = meta.clone();
            view.composer.input.update(cx, |input, cx| input.set_value("unsent draft", window, cx));
            view.request_restart_agent_session(meta.id, cx);
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::RestartSession(id)) if id == meta.id));
            view.update(Update::RestartedSession { session: meta.id, result: Err("restart rejected".into()) }, window, cx);
            assert!(!view.sessions.pending.restarting.contains(&meta.id));
            assert_eq!(view.composer.input.read(cx).value(), "unsent draft");
            view.request_restart_agent_session(meta.id, cx);
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::RestartSession(id)) if id == meta.id));
            meta.status = SessionStatus::Idle;
            meta.revision = Cursor::from_raw(1);
            view.update(Update::RestartedSession { session: meta.id, result: Ok(meta.clone().into()) }, window, cx);
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::Watch { session, .. }) if session == meta.id));
            assert_eq!(view.composer.input.read(cx).value(), "unsent draft");
            assert_eq!(view.selected.stream, StreamState::Loading);
        });
    }).expect("retry and reattach");
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn restart_send_failure_releases_pending_state(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Stopped;
    meta.initialization = Some(listing().initialization);
    attach(&fixture, meta.clone(), cx);
    fixture.commands.close();
    cx.update(|cx| {
        fixture.view.update(cx, |view, cx| {
            view.request_restart_agent_session(meta.id, cx);
            assert!(!view.sessions.pending.restarting.contains(&meta.id));
        })
    });
}

#[gpui_kit::test]
fn ctrl_m_reopens_session_management_from_the_composer(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let history = open_history(&fixture, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        fixture
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        window.render_frame(cx);
        window.press("ctrl-m", cx);
    })
    .expect("management shortcut");
    cx.run_until_parked();
    cx.update_window(history.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("agent-history-window").visible());
        assert!(window.is_window_active());
    })
    .expect("existing management window activated");
}

#[gpui_kit::test]
fn import_window_is_independent_and_import_keeps_the_outgoing_composer(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut previous = session();
    previous.cwd = Some("/tmp".into());
    attach(&fixture, previous.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("keep my draft", window, cx));
        });
    })
    .expect("draft");
    while fixture.commands.try_recv().is_ok() {}
    let mut history = open_history(&fixture, cx);
    assert_ne!(history.window.window_id(), fixture.window.window_id());
    let Command::AgentHistory { epoch, agent, cwd } =
        history.commands.try_recv().expect("discovery")
    else {
        panic!("expected discovery")
    };
    assert_eq!(agent, "history-agent");
    assert_eq!(cwd, Some("/tmp".into()));
    assert!(
        fixture.commands.try_recv().is_err(),
        "opening a history window does not replace the chat or send commands"
    );
    cx.update(|cx| {
        assert_eq!(fixture.view.read(cx).selected.id, Some(previous.id));
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "keep my draft"
        );
    });
    history
        .updates
        .try_send(Update::AgentHistory {
            epoch,
            result: Ok(listing()),
        })
        .expect("history page");
    cx.run_until_parked();
    cx.update_window(history.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-history-import-0-0", cx);
    })
    .expect("import button");
    let Command::ImportHistory { epoch, agent, info } =
        history.commands.try_recv().expect("import")
    else {
        panic!("expected import")
    };
    assert_eq!(agent, "history-agent");
    assert_eq!(info.session_id.to_string(), "external/id");
    let mut imported = session();
    imported.agent = "history-agent".into();
    history
        .updates
        .try_send(Update::ImportedHistory {
            epoch,
            result: Ok(SessionSnapshot::from(imported.clone())),
        })
        .expect("receipt");
    cx.run_until_parked();
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(imported.id));
        assert_eq!(view.sessions.drafts[&previous.id].text, "keep my draft");
        assert!(!view.sessions.is_draft(imported.id));
    });
    while let Ok(command) = fixture.commands.try_recv() {
        assert!(!matches!(
            command,
            Command::Prompt { .. } | Command::Create(_)
        ));
    }
}

#[gpui_kit::test]
fn closing_the_parent_preserves_the_import_windows_connection(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut history = open_history(&fixture, cx);
    let Command::AgentHistory { epoch, .. } = history.commands.try_recv().expect("discovery")
    else {
        panic!("discovery")
    };
    cx.update_window(fixture.window.into(), |_, window, _| window.remove_window())
        .expect("close parent");
    cx.run_until_parked();
    history
        .updates
        .try_send(Update::AgentHistory {
            epoch,
            result: Ok(listing()),
        })
        .expect("page after parent close");
    cx.run_until_parked();
    cx.update_window(history.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-history-import-0-0", cx);
    })
    .expect("independently import");
    let Command::ImportHistory { epoch, .. } = history.commands.try_recv().expect("import") else {
        panic!("import")
    };
    history
        .updates
        .try_send(Update::ImportedHistory {
            epoch,
            result: Ok(SessionSnapshot::from(session())),
        })
        .expect("receipt");
    cx.run_until_parked();
    cx.update_window(history.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("agent-history-window").bounds().size.height > px(0.));
    })
    .expect("history still open");
}

#[gpui_kit::test]
fn closing_history_does_not_close_or_reset_the_chat(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let previous = session();
    attach(&fixture, previous.clone(), cx);
    let history = open_history(&fixture, cx);
    cx.update_window(history.window.into(), |_, window, _| window.remove_window())
        .expect("close history");
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(fixture.view.read(cx).selected.id, Some(previous.id)));
    assert!(
        history.updates.is_closed(),
        "closing the window releases its update receiver"
    );
}

#[test]
fn conversation_history_icon_is_bundled_instead_of_reusing_search() {
    use gpui_kit::AssetSource;
    assert!(
        crate::assets::Assets
            .load("icons/list-clock.svg")
            .expect("asset")
            .is_some()
    );
}
