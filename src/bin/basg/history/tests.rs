use super::*;
use bas::model::{Owner, SessionMeta};
use gpui_kit::{TestAppContext, WeakEntity, WindowHandle, component::Root, test::TestWindowExt};
use std::cell::RefCell;
use tokio::sync::mpsc;

struct Fixture {
    window: WindowHandle<Root>,
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
    let server = "127.0.0.1:7337".parse().expect("server");
    let (window, browser) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
        window::bind_keys(cx);
        let handle = window::open_connected(
            OpenRequest {
                server,
                agents: agents(),
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
fn agent_and_primary_workspace_filters_are_exact_and_cross_provider_ids_do_not_collide(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let (initial, agent, cwd) = query(&mut fixture);
    assert_eq!(agent, "codex-acp");
    assert_eq!(cwd, Some("/repo".into()));
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: initial,
            result: Ok(listing(vec![row("same-id", "/repo")])),
        },
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                browser.select_agent(None, cx);
                assert_eq!(
                    browser
                        .agents
                        .iter()
                        .map(|agent| agent.id.as_str())
                        .collect::<Vec<_>>(),
                    ["codex-acp", "pi-acp"]
                );
            })
            .expect("all Agents")
    });
    let first = query(&mut fixture);
    let second = query(&mut fixture);
    for (request, agent, cwd) in [first, second] {
        assert_eq!(cwd, Some("/repo".into()));
        let mut extra_root = row("additional-only", "/other");
        extra_root.additional_directories = vec!["/repo".into()];
        deliver(
            &fixture,
            Update::AgentHistory {
                epoch: request,
                result: Ok(listing(vec![row("same-id", "/repo"), extra_root])),
            },
            cx,
        );
        assert!(["codex-acp", "pi-acp"].contains(&agent.as_str()));
    }
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                for (agent, page) in &browser.pages {
                    assert_eq!(
                        browser.visible_rows(agent, page, "").len(),
                        1,
                        "additional directories do not make a session match its primary cwd"
                    );
                }
                browser.import(
                    Target {
                        generation: browser.generation,
                        agent: "pi-acp".into(),
                        info: row("same-id", "/repo"),
                    },
                    cx,
                );
            })
            .expect("provider scoped import")
    });
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::ImportHistory { agent, info, .. })
        if agent == "pi-acp" && info.session_id.to_string() == "same-id")
    );
    assert!(fixture.opened.borrow().is_empty());
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
fn rows_are_grouped_by_agent_and_real_buttons_import_the_original_provider(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Ok(listing(vec![row("external", "/repo")])),
        },
        cx,
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("agent-history-group-0").bounds().size.height > gpui_kit::px(0.));
        window.click("agent-history-import-0-0", cx);
    })
    .expect("click import");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::ImportHistory { agent, info, .. })
        if agent == "codex-acp" && info.session_id.to_string() == "external")
    );
}

#[gpui_kit::test]
fn relative_time_sorting_keeps_unknown_dates_last_without_fake_now(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let rows = listing(vec![
        row("unknown", "/repo").updated_at("invalid"),
        row("older", "/repo").updated_at("2026-10-15T10:00:00Z"),
        row("newer", "/repo").updated_at("2026-10-16T18:00:00+08:00"),
    ]);
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| {
                let visible = browser.visible_rows("codex-acp", &rows, "");
                assert_eq!(
                    visible
                        .iter()
                        .map(|info| info.session_id.to_string())
                        .collect::<Vec<_>>(),
                    ["newer", "older", "unknown"]
                );
            })
            .expect("time order")
    });
}

#[gpui_kit::test]
fn catalog_removal_forgets_old_bindings_but_not_newer_import_receipts(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    let info = row("external", "/repo");
    let snapshot = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "codex-acp".into(),
        Owner::current(),
    ));
    let id = snapshot.meta.id;
    let mut page = listing(vec![info.clone()]);
    page.imported.insert("external".into(), id.to_string());
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
            .update(cx, |browser, _| browser.merge_session(snapshot.clone()))
            .expect("receipt")
    });
    deliver(
        &fixture,
        Update::Catalog(Ok(connection::Catalog {
            agents: agents(),
            sessions: Vec::new(),
        })),
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| {
                assert!(
                    browser.imported("codex-acp", &info).is_some(),
                    "old catalog cannot erase a newer receipt"
                );
            })
            .expect("stale catalog")
    });
    deliver(
        &fixture,
        Update::Catalog(Ok(connection::Catalog {
            agents: agents(),
            sessions: vec![snapshot],
        })),
        cx,
    );
    deliver(
        &fixture,
        Update::Catalog(Ok(connection::Catalog {
            agents: agents(),
            sessions: Vec::new(),
        })),
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| {
                assert!(
                    browser.imported("codex-acp", &info).is_none(),
                    "confirmed removal must not keep a stale view-record action"
                );
            })
            .expect("removed binding")
    });
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

#[gpui_kit::test]
fn long_workspace_filters_and_action_icons_fit_the_minimum_window(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Ok(listing(Vec::new())),
        },
        cx,
    );
    let cwd = PathBuf::from(format!(
        "/projects/{}/primary",
        "long-workspace-".repeat(20)
    ));
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                browser.select_workspace(Some(cwd.clone()), cx)
            })
            .expect("long cwd")
    });
    let (request, _, _) = query(&mut fixture);
    let info = acp::SessionInfo::new("external", cwd)
        .title("Imported history")
        .updated_at("2026-10-16T10:00:00Z");
    let mut local = SessionMeta::new(SessionId::new(), "codex-acp".into(), Owner::current());
    local.status = SessionStatus::Idle;
    let mut page = listing(vec![info]);
    page.imported
        .insert("external".into(), local.id.to_string());
    deliver(
        &fixture,
        Update::Catalog(Ok(connection::Catalog {
            agents: agents(),
            sessions: vec![local.into()],
        })),
        cx,
    );
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Ok(page),
        },
        cx,
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(780.), gpui_kit::px(480.)));
        window.render_frame(cx);
        let bounds = window.find("agent-history-window").bounds();
        for id in [
            "agent-history-workspace-filter",
            "agent-history-refresh",
            "agent-history-import-0-0",
            "agent-history-close-0-0",
            "agent-history-delete-0-0",
        ] {
            let control = window.find(id).bounds();
            assert!(
                control.left() >= bounds.left() && control.right() <= bounds.right(),
                "{id} exceeds history window width: {control:?} outside {bounds:?}"
            );
            assert!(
                control.bottom() <= bounds.bottom(),
                "{id} exceeds history window height"
            );
        }
    })
    .expect("minimum history window");
}

#[gpui_kit::test]
fn reopening_history_reuses_its_native_window_and_applies_requested_filters(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let (commands, _unused) = mpsc::unbounded_channel();
    let (_updates, events) = mpsc::channel(1);
    let reopened = cx.update(|cx| {
        window::open_connected(
            OpenRequest {
                server: "127.0.0.1:7337".parse().expect("server"),
                agents: agents(),
                agent: Some("pi-acp".into()),
                cwd: None,
                workspaces: Vec::new(),
                open_session: Rc::new(|_, _| true),
            },
            Connection::for_test(commands),
            events,
            cx,
        )
        .expect("reopen")
    });
    assert_eq!(fixture.window.window_id(), reopened.window_id());
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, _| {
                assert_eq!(browser.agent.as_deref(), Some("pi-acp"));
                assert_eq!(browser.cwd, None);
            })
            .expect("reconfigured scope")
    });
}
