use super::*;
use gpui_kit::{px, test::TestWindowExt};

fn rendered(
    fixture: &Fixture,
    present: &[&'static str],
    absent: &[&'static str],
    cx: &mut TestAppContext,
) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        for id in present {
            let element = window.find(*id);
            assert!(element.visible(), "{id} should be visible");
            assert!(element.bounds().size.height > px(0.));
            if matches!(*id, "agent-history-loading" | "agent-history-pending") {
                let spinner = window.find(format!("{id}-spinner"));
                assert!(spinner.visible());
                assert_eq!(spinner.bounds().size.width, px(crate::ui::ICON));
            }
        }
        for id in absent {
            assert!(window.try_find(*id).is_none(), "{id} should be absent");
        }
    })
    .expect("render history browser");
}

#[gpui_kit::test]
fn initial_catalog_wait_ends_in_genuine_empty_state(cx: &mut TestAppContext) {
    let fixture = fixture_with_agents(Vec::new(), cx);
    rendered(
        &fixture,
        &["agent-history-loading"],
        &["agent-history-empty"],
        cx,
    );
    deliver(
        &fixture,
        Update::Catalog(Ok(connection::Catalog {
            agents: Vec::new(),
            sessions: Vec::new(),
        })),
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-empty"],
        &["agent-history-loading"],
        cx,
    );
}

#[gpui_kit::test]
fn failed_catalog_stops_spinner_without_false_empty_state(cx: &mut TestAppContext) {
    let fixture = fixture_with_agents(Vec::new(), cx);
    deliver(&fixture, Update::Catalog(Err("catalog failure".into())), cx);
    rendered(
        &fixture,
        &[
            "agent-history-connection-error",
            "agent-history-connection-error-copy",
        ],
        &["agent-history-loading", "agent-history-empty"],
        cx,
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.click("agent-history-connection-error-copy", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("catalog failure".into())
        );
    })
    .expect("copy catalog error");
}

#[gpui_kit::test]
fn discovery_empty_reload_and_failure_render_distinct_states(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    rendered(
        &fixture,
        &["agent-history-loading"],
        &["agent-history-empty"],
        cx,
    );
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Ok(listing(Vec::new())),
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-empty"],
        &["agent-history-loading"],
        cx,
    );
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| browser.reload(cx))
            .expect("reload")
    });
    rendered(
        &fixture,
        &["agent-history-loading"],
        &["agent-history-empty"],
        cx,
    );
    let (request, _, _) = query(&mut fixture);
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: request,
            result: Err("discovery failure".into()),
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-error-0", "agent-history-error-0-copy"],
        &["agent-history-loading", "agent-history-empty"],
        cx,
    );
}

#[gpui_kit::test]
fn partial_groups_remain_visible_until_remaining_discovery_finishes(cx: &mut TestAppContext) {
    let mut fixture = fixture_with_agents(Vec::new(), cx);
    deliver(
        &fixture,
        Update::Catalog(Ok(connection::Catalog {
            agents: agents(),
            sessions: Vec::new(),
        })),
        cx,
    );
    let (first, _, _) = query(&mut fixture);
    let (second, _, _) = query(&mut fixture);
    assert!(fixture.commands.try_recv().is_err());
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: first,
            result: Ok(listing(vec![row("first", "/repo")])),
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-loading", "agent-history-group-0"],
        &["agent-history-empty"],
        cx,
    );
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: second,
            result: Ok(listing(vec![row("second", "/repo")])),
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-group-0", "agent-history-group-1"],
        &["agent-history-loading", "agent-history-empty"],
        cx,
    );
}

#[gpui_kit::test]
fn stale_generation_cannot_end_latest_loading_state(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (old, _, _) = query(&mut fixture);
    cx.update(|cx| {
        fixture
            .browser
            .update(cx, |browser, cx| {
                browser.select_workspace(Some("/other".into()), cx);
                browser.select_agent(None, cx);
                assert_eq!(browser.queries.len(), MAX_DISCOVERIES);
            })
            .expect("filters")
    });
    let (stale, _, _) = query(&mut fixture);
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: old,
            result: Ok(listing(vec![row("old", "/repo")])),
        },
        cx,
    );
    let (latest_first, _, _) = query(&mut fixture);
    rendered(
        &fixture,
        &["agent-history-loading"],
        &["agent-history-group-0", "agent-history-empty"],
        cx,
    );
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: stale,
            result: Err("stale failure".into()),
        },
        cx,
    );
    let (latest_second, _, _) = query(&mut fixture);
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: latest_first,
            result: Ok(listing(Vec::new())),
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-loading"],
        &["agent-history-empty", "agent-history-error-0"],
        cx,
    );
    deliver(
        &fixture,
        Update::AgentHistory {
            epoch: latest_second,
            result: Ok(listing(Vec::new())),
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-empty"],
        &["agent-history-loading"],
        cx,
    );
}

#[gpui_kit::test]
fn import_status_animates_without_hiding_rows_and_finishes_on_error(cx: &mut TestAppContext) {
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
        window.click("agent-history-import-0-0", cx);
    })
    .expect("import action");
    let Command::ImportHistory { epoch, .. } = fixture.commands.try_recv().expect("import command")
    else {
        panic!("expected import");
    };
    rendered(
        &fixture,
        &["agent-history-pending", "agent-history-group-0"],
        &["agent-history-loading", "agent-history-empty"],
        cx,
    );
    deliver(
        &fixture,
        Update::ImportedHistory {
            epoch,
            result: Err("import failure".into()),
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-group-0", "agent-history-error-0-copy"],
        &["agent-history-pending"],
        cx,
    );
}

fn open_saved_record(archived: bool, cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let (request, _, _) = query(&mut fixture);
    let mut snapshot = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "codex-acp".into(),
        Owner::current(),
    ));
    snapshot.meta.status = SessionStatus::Stopped;
    snapshot.meta.archived = archived;
    let id = snapshot.meta.id;
    let mut page = listing(vec![row("saved", "/repo")]);
    page.imported.insert("saved".into(), id.to_string());
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
                cx.notify();
            })
            .expect("saved record")
    });
    let action = if archived {
        "agent-history-restore-0-0"
    } else {
        "agent-history-restart-0-0"
    };
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(action, cx);
    })
    .expect("open saved record");
    match fixture.commands.try_recv().expect("record operation") {
        Command::Archive {
            session,
            archived: false,
        } if archived => assert_eq!(session, id),
        Command::RestartSession(session) if !archived => assert_eq!(session, id),
        _ => panic!("unexpected record operation"),
    }
    rendered(
        &fixture,
        &["agent-history-pending", "agent-history-group-0", action],
        &["agent-history-loading"],
        cx,
    );
    // An unrelated receipt cannot stop the active operation's feedback.
    deliver(
        &fixture,
        Update::RestartedSession {
            session: SessionId::new(),
            result: Err("unrelated".into()),
        },
        cx,
    );
    rendered(&fixture, &["agent-history-pending"], &[], cx);
    snapshot.meta.archived = false;
    snapshot.meta.status = SessionStatus::Idle;
    let result = Ok(snapshot);
    deliver(
        &fixture,
        if archived {
            Update::Archived {
                session: id,
                result,
            }
        } else {
            Update::RestartedSession {
                session: id,
                result,
            }
        },
        cx,
    );
    rendered(
        &fixture,
        &["agent-history-group-0"],
        &["agent-history-pending"],
        cx,
    );
    assert_eq!(*fixture.opened.borrow(), vec![id]);
}

#[gpui_kit::test]
fn restart_status_keeps_rows_visible_until_matching_receipt(cx: &mut TestAppContext) {
    open_saved_record(false, cx);
}

#[gpui_kit::test]
fn restore_status_keeps_rows_visible_until_matching_receipt(cx: &mut TestAppContext) {
    open_saved_record(true, cx);
}
