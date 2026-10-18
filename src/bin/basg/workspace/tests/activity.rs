use super::*;
use bas::model::{RuntimeFailure, SessionSnapshot, TurnId};
use std::time::Duration;

fn working(name: &str) -> SessionMeta {
    let mut meta = session();
    meta.title = Some(name.into());
    meta.status = SessionStatus::Working;
    meta.revision = Cursor::from_raw(1);
    meta
}

fn catalog(fixture: &Fixture, sessions: Vec<SessionSnapshot>, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions,
                })),
                window,
                cx,
            );
            cx.notify();
        });
    })
    .expect("catalog");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn protocol_children_are_counted_while_the_parent_is_idle_and_never_flattened_into_its_answer(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let parent = session();
    attach(&fixture, parent.clone(), cx);
    let running = || bas::acp::StateUpdate::Running(bas::acp::RunningStateUpdate::new());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for (index, kind) in [
                EventKind::Acp(bas::acp::SessionUpdate::SubagentUpdate(
                    bas::acp::SubagentUpdate::new("child")
                        .title("Researcher")
                        .description("Read source")
                        .state(running()),
                )),
                EventKind::Subagent(bas::acp::SessionNotification::new(
                    "child",
                    bas::acp::SessionUpdate::SubagentUpdate(
                        bas::acp::SubagentUpdate::new("nested")
                            .title("Nested")
                            .state(running()),
                    ),
                )),
                EventKind::Subagent(bas::acp::SessionNotification::new(
                    "nested",
                    bas::acp::SessionUpdate::AgentMessageChunk(bas::acp::ContentChunk::new(
                        "child-only content".into(),
                    )),
                )),
            ]
            .into_iter()
            .enumerate()
            {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: Event::new(parent.id, Cursor::from_raw(index as u64 + 1), kind),
                    },
                    window,
                    cx,
                );
            }
            assert_eq!(view.current().expect("parent").status, SessionStatus::Idle);
            assert_eq!(view.activity_count(), 2);
            assert!(view.running_sessions().is_empty());
            assert!(
                view.selected
                    .timeline
                    .conversation
                    .as_ref()
                    .expect("conversation")
                    .items
                    .is_empty()
            );
            cx.notify();
        });
        window.render_frame(cx);
        window.hover("agent-picker", cx);
    })
    .expect("receive protocol subagents");
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let popup = cx.update(|cx| {
        fixture
            .view
            .read(cx)
            .window
            .reply_preview
            .as_ref()
            .expect("panel")
            .window
            .expect("popup")
    });
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window
                .find(format!("agent-activity-title-{}-child-child", parent.id))
                .label(),
            Some("Researcher")
        );
        assert_eq!(
            window
                .find(format!("agent-activity-title-{}-child-nested", parent.id))
                .label(),
            Some("Nested")
        );
        assert!(
            window
                .try_find(format!("agent-activity-title-{}", parent.id))
                .is_none()
        );
    })
    .expect("child rows");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        parent.id,
                        Cursor::from_raw(4),
                        EventKind::Acp(bas::acp::SessionUpdate::SubagentUpdate(
                            bas::acp::SubagentUpdate::new("child").state(
                                bas::acp::StateUpdate::Idle(bas::acp::IdleStateUpdate::new()),
                            ),
                        )),
                    ),
                },
                window,
                cx,
            );
            assert_eq!(
                view.activity_count(),
                1,
                "parent child becoming idle does not finish its descendant"
            );
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        parent.id,
                        Cursor::from_raw(5),
                        EventKind::Subagent(bas::acp::SessionNotification::new(
                            "child",
                            bas::acp::SessionUpdate::SubagentUpdate(
                                bas::acp::SubagentUpdate::new("nested").state(
                                    bas::acp::StateUpdate::RequiresAction(
                                        bas::acp::RequiresActionStateUpdate::new(),
                                    ),
                                ),
                            ),
                        )),
                    ),
                },
                window,
                cx,
            );
            assert_eq!(view.activity_count(), 1);
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        parent.id,
                        Cursor::from_raw(6),
                        EventKind::Failed {
                            message: "connection ended".into(),
                        },
                    ),
                },
                window,
                cx,
            );
            assert_eq!(view.activity_count(), 0);
        });
    })
    .expect("independent child lifecycles");
}

#[gpui_kit::test]
fn a_foreground_agent_alone_has_no_activity_badge(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let current = working("only foreground");
    attach(&fixture, current.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(fixture.view.read(cx).running_sessions().is_empty());
        assert_ne!(
            window.find("agent-picker-hint").role(),
            Some(gpui_kit::Role::Status)
        );
        window.hover("agent-picker", cx);
    })
    .expect("foreground only");
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|cx| assert!(fixture.view.read(cx).window.reply_preview.is_none()));
}

#[gpui_kit::test]
fn activity_counts_working_sessions_not_agent_types_and_excludes_failed_runtimes(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let current = session();
    attach(&fixture, current.clone(), cx);
    let first = working("first");
    let second = working("same agent, another session");
    let mut failed = SessionSnapshot::from(working("failed"));
    failed.runtime.failure = Some(RuntimeFailure::Persistence {
        message: "disk full".into(),
    });
    catalog(
        &fixture,
        vec![
            current.clone().into(),
            first.clone().into(),
            second.clone().into(),
            failed,
        ],
        cx,
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).running_sessions().len(), 2);
        let label = window
            .find("agent-picker-hint")
            .label()
            .expect("count")
            .to_owned();
        assert_eq!(crate::i18n::visible_text(&label), "后台智能体：2");
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        current.id,
                        Cursor::from_raw(2),
                        EventKind::TurnStarted {
                            turn: TurnId::new(),
                        },
                    ),
                },
                window,
                cx,
            );
            assert_eq!(
                view.running_sessions().len(),
                2,
                "the foreground session is never counted"
            );
            view.select(first.id, window, cx);
            let ids: Vec<_> = view
                .running_sessions()
                .iter()
                .map(|session| session.id)
                .collect();
            assert!(!ids.contains(&first.id));
            assert!(
                ids.contains(&current.id),
                "previous foreground is now background work"
            );
            view.navigation.view = workspace_navigation::View::Archived;
            view.navigation.search.update(cx, |input, cx| {
                input.set_value("no matching rows", window, cx)
            });
            assert_eq!(
                view.running_sessions().len(),
                2,
                "sidebar filters do not change running count"
            );
            view.backend.connection_lost("offline".into());
            assert!(
                view.running_sessions().is_empty(),
                "do not claim stale work is still running"
            );
        });
    })
    .expect("running counts");
}

#[gpui_kit::test]
fn activity_hover_is_live_scrollable_and_preserves_the_composer_in_both_modes(
    cx: &mut TestAppContext,
) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let current = working("current work");
        let mut background = working("background work");
        attach(&fixture, current.clone(), cx);
        catalog(
            &fixture,
            vec![current.clone().into(), background.clone().into()],
            cx,
        );
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("keep my draft", window, cx);
                    input.focus(window, cx);
                });
            });
            if compact {
                window.press("ctrl-shift-m", cx);
                window.bounds_changed(cx);
            }
            window.render_frame(cx);
            window.hover("agent-picker", cx);
        })
        .expect("hover disabled agent picker");
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        let popup = cx.update(|cx| {
            fixture
                .view
                .read(cx)
                .window
                .reply_preview
                .as_ref()
                .expect("activity preview")
                .window
                .expect("native hover panel")
        });
        cx.update_window(popup, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("agent-activity-scroll").visible());
            assert!(
                window
                    .try_find(format!("agent-activity-title-{}", current.id))
                    .is_none()
            );
            assert_eq!(
                window
                    .find(format!("agent-activity-title-{}", background.id))
                    .label(),
                Some("background work")
            );
            window.hover("agent-activity-panel", cx);
        })
        .expect("hover panel");
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.update(|cx| assert!(fixture.view.read(cx).window.reply_preview.is_some()));
        background.title = Some("updated work".into());
        background.revision = Cursor::from_raw(2);
        catalog(
            &fixture,
            vec![current.clone().into(), background.clone().into()],
            cx,
        );
        cx.update_window(popup, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window
                    .find(format!("agent-activity-title-{}", background.id))
                    .label(),
                Some("updated work")
            );
        })
        .expect("live title");
        background.status = SessionStatus::Idle;
        background.revision = Cursor::from_raw(3);
        catalog(
            &fixture,
            vec![current.clone().into(), background.clone().into()],
            cx,
        );
        cx.update_window(popup, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .try_find(format!("agent-activity-title-{}", background.id))
                    .is_none()
            );
            assert_eq!(
                window
                    .find("agent-activity-heading")
                    .label()
                    .map(crate::i18n::visible_text)
                    .as_deref(),
                Some("后台智能体：0")
            );
        })
        .expect("completed work disappears");
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                assert_eq!(view.composer.input.read(cx).value(), "keep my draft");
                assert_eq!(view.selected.id, Some(current.id));
                assert!(
                    view.composer
                        .input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
                view.hover_activity(false, gpui_kit::Bounds::default(), window, cx);
            });
        })
        .expect("hover does not focus or select");
        cx.executor().advance_clock(Duration::from_millis(250));
        cx.run_until_parked();
        cx.update(|cx| assert!(fixture.view.read(cx).window.reply_preview.is_none()));
        while let Ok(command) = fixture.commands.try_recv() {
            assert!(
                matches!(command, Command::Watch { .. } | Command::Foreground(_)),
                "hover must not mutate or send prompts"
            );
        }
    }
}
