use super::*;
use bas::model::{Owner, SessionMeta};
use gpui_kit::{TestAppContext, WeakEntity};
use std::cell::RefCell;
use tokio::sync::mpsc;

mod loading;

struct Fixture {
    window: gpui_kit::WindowHandle<gpui_kit::component::Root>,
    browser: WeakEntity<Browser>,
    commands: mpsc::UnboundedReceiver<Command>,
    opened: Rc<RefCell<Vec<SessionId>>>,
    _updates: mpsc::Sender<Update>,
    _settings: tempfile::TempDir,
}

fn agents() -> Vec<AgentSummary> {
    ["pi-acp", "embedded", "codex-acp"]
        .into_iter()
        .map(|id| AgentSummary {
            id: id.into(),
            kind: if id == "embedded" { "embedded" } else { "acp" }.into(),
            command: String::new(),
        })
        .collect()
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    fixture_with_agents(agents(), cx)
}

fn fixture_with_agents(agents: Vec<AgentSummary>, cx: &mut TestAppContext) -> Fixture {
    let settings = tempfile::tempdir().expect("settings");
    let path = settings.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"appearance":{"theme_source":"builtin","theme_mode":"dark"}}"#,
    )
    .expect("settings");
    let (commands, receiver) = mpsc::unbounded_channel();
    let (updates, events) = mpsc::channel(64);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let recorded = opened.clone();
    let server: bas::service::tls::ConnectionOptions = "127.0.0.1".parse().expect("server");
    let (window, browser) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
        window::bind_keys(cx);
        let handle = window::open_connected(
            OpenRequest {
                server: server.clone(),
                agents,
                agent: Some("codex-acp".into()),
                cwd: Some("/repo".into()),
                workspaces: vec!["/repo".into(), "/other".into()],
                open_session: Rc::new(move |snapshot, _| {
                    recorded.borrow_mut().push(snapshot.meta.id);
                    true
                }),
            },
            Connection::for_test(commands),
            events,
            cx,
        )
        .expect("history window");
        (handle, window::browser(server, cx))
    });
    Fixture {
        window,
        browser,
        commands: receiver,
        opened,
        _updates: updates,
        _settings: settings,
    }
}

fn listing(rows: Vec<acp::SessionInfo>) -> connection::AgentHistory {
    connection::AgentHistory {
        sessions: acp::ListSessionsResponse::new(rows),
        initialization: acp::InitializeResponse::new(
            agent_client_protocol::schema::ProtocolVersion::V1,
        )
        .agent_capabilities(
            acp::AgentCapabilities::new()
                .load_session(true)
                .session_capabilities(
                    acp::SessionCapabilities::new()
                        .delete(acp::SessionDeleteCapabilities::new())
                        .close(acp::SessionCloseCapabilities::new()),
                ),
        ),
        imported: HashMap::new(),
    }
}

fn row(id: &str, cwd: &str) -> acp::SessionInfo {
    acp::SessionInfo::new(id.to_owned(), cwd)
        .title(format!("History {id}"))
        .updated_at("2026-10-16T10:00:00Z")
}

fn query(fixture: &mut Fixture) -> (u64, String, Option<PathBuf>) {
    let Command::AgentHistory { epoch, agent, cwd } =
        fixture.commands.try_recv().expect("discovery")
    else {
        panic!("discovery must not create, import or prompt")
    };
    (epoch, agent, cwd)
}

fn deliver(fixture: &Fixture, update: Update, cx: &mut TestAppContext) {
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |this, cx| this.update(update, cx))
            .expect("live browser")
    });
}

#[gpui_kit::test]
fn changing_workspace_passes_cwd_to_rpc_and_old_pages_cannot_repopulate_the_view(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let (old, _, _) = query(&mut fixture);
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                browser.select_workspace(Some("/other".into()), cx);
            })
            .expect("workspace")
    });
    let (current, agent, cwd) = query(&mut fixture);
    assert_eq!(agent, "codex-acp");
    assert_eq!(cwd, Some("/other".into()));
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: old,
            result: Ok(listing(vec![row("old", "/repo")])),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| assert!(browser.pages.is_empty()))
            .expect("browser")
    });
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: current,
            result: Ok(listing(vec![row("new", "/other")])),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                assert_eq!(
                    browser.pages["codex-acp"].sessions.sessions[0]
                        .session_id
                        .to_string(),
                    "new"
                );
                browser.select_workspace(None, cx);
            })
            .expect("all workspaces")
    });
    assert_eq!(query(&mut fixture).2, None);
}

#[gpui_kit::test]
fn rapid_filter_changes_keep_discovery_processes_bounded(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (old, _, _) = query(&mut fixture);
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                browser.select_agent(None, cx);
                for index in 0..20 {
                    browser.select_workspace(Some(format!("/workspace-{index}").into()), cx);
                }
                assert_eq!(browser.queries.len(), MAX_DISCOVERIES);
            })
            .expect("rapid filters")
    });
    let (_second, _, _) = query(&mut fixture);
    assert!(fixture.commands.try_recv().is_err());
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: old,
            result: Ok(listing(Vec::new())),
        },
        cx,
    );
    assert_eq!(query(&mut fixture).2, Some("/workspace-19".into()));
}

#[gpui_kit::test]
fn stale_import_controls_and_receipts_cannot_retarget_a_new_filter(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    let info = row("external", "/repo");
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Ok(listing(vec![info.clone()])),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                let target = Target {
                    generation: browser.generation,
                    agent: "codex-acp".into(),
                    info,
                };
                browser.import(target.clone(), cx);
                browser.select_agent(Some("pi-acp".into()), cx);
                browser.import(target, cx);
            })
            .expect("filter during import")
    });
    let Command::ImportHistory { epoch, agent, .. } = fixture.commands.try_recv().expect("import")
    else {
        panic!("import")
    };
    assert_eq!(agent, "codex-acp");
    let _query = query(&mut fixture);
    assert!(fixture.commands.try_recv().is_err());
    let snapshot = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "codex-acp".into(),
        Owner::current(),
    ));
    let id = snapshot.meta.id;
    deliver(
        &fixture,
        Update::ImportedHistory {
            epoch,
            result: Ok(snapshot),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| {
                assert!(browser.sessions.contains_key(&id));
                assert!(browser.operation.is_none());
                assert!(browser.pages.is_empty());
            })
            .expect("late receipt")
    });
    assert!(fixture.opened.borrow().is_empty());
}

#[gpui_kit::test]
fn primary_action_rejects_stale_controls_after_a_state_change(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    let info = row("external", "/repo");
    let mut snapshot = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "codex-acp".into(),
        Owner::current(),
    ));
    snapshot.meta.status = SessionStatus::Idle;
    let id = snapshot.meta.id;
    let mut page = listing(vec![info.clone()]);
    page.imported
        .insert(info.session_id.to_string(), id.to_string());
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Ok(page),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                browser.merge_session(snapshot.clone());
                let target = Target {
                    generation: browser.generation,
                    agent: "codex-acp".into(),
                    info,
                };
                assert!(browser.primary_action(&target.agent, &target.info) == PrimaryAction::Open);
                snapshot.meta.archived = true;
                snapshot.meta.status = SessionStatus::Stopped;
                snapshot.meta.revision = bas::model::Cursor::from_raw(1);
                browser.merge_session(snapshot);
                browser.activate(target.clone(), PrimaryAction::Open, cx);
                browser.activate(target.clone(), PrimaryAction::Restart, cx);
                assert!(
                    browser.operation.is_none(),
                    "stale actions must not start or restore a session"
                );
                browser.activate(target, PrimaryAction::Restore, cx);
            })
            .expect("state-sensitive primary action")
    });
    assert!(fixture.opened.borrow().is_empty());
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Archive { session, archived: false }) if session == id)
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn native_deletion_receipts_are_correlated_and_keep_local_transcripts(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    let info = row("external", "/repo");
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Ok(listing(vec![info.clone()])),
        },
        cx,
    );
    let local = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "codex-acp".into(),
        Owner::current(),
    ));
    let id = local.meta.id;
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| {
                browser.merge_session(local);
                browser.operation = Some(Operation::Delete {
                    request: 100,
                    target: Target {
                        generation: browser.generation,
                        agent: "codex-acp".into(),
                        info,
                    },
                });
            })
            .expect("pending deletion")
    });
    deliver(
        &fixture,
        Update::DeletedAgentHistory {
            epoch: 99,
            id: "external".into(),
            result: Ok(()),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| assert!(browser.operation.is_some()))
            .expect("mismatched receipt")
    });
    deliver(
        &fixture,
        Update::DeletedAgentHistory {
            epoch: 100,
            id: "external".into(),
            result: Ok(()),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| {
                assert!(browser.operation.is_none());
                assert!(browser.pages["codex-acp"].sessions.sessions.is_empty());
                assert!(browser.sessions.contains_key(&id));
            })
            .expect("delete scope")
    });
    assert!(fixture.opened.borrow().is_empty());
}
