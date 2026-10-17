use super::*;

#[gpui_kit::test]
fn sidebar_can_archive_and_compact_mode_can_restore(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.toggle_sidebar(window, cx);
        });
        window.render_frame(cx);
        assert!(
            window
                .try_find(format!("header-archive-{}", meta.id))
                .is_none()
        );
        window.press("ctrl-b", cx);
        window.render_frame(cx);
        window.hover(format!("session-{}", meta.id), cx);
        window.click(format!("archive-{}", meta.id), cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Archived {
                    session: meta.id,
                    result: Ok(archived(meta.clone(), true, 2).into()),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert!(
            window
                .find(format!("restore-archived-{}", meta.id))
                .visible()
        );
        fixture.view.update(cx, |view, cx| {
            view.toggle_mode(window, cx);
        });
        window.render_frame(cx);
        window.click(format!("compact-restore-{}", meta.id), cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(meta.id));
    })
    .expect("archive navigation through the sidebar");
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::Archive { session, archived: true }) if session == meta.id
    ));
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::Archive { session, archived: false }) if session == meta.id
    ));
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn search_enter_never_selects_a_thread_hidden_by_the_archive_filter(cx: &mut TestAppContext) {
    for archived_list in [false, true] {
        let mut fixture = fixture(cx);
        let mut visible = session();
        visible.archived = archived_list;
        visible.title = Some("visible".into());
        let mut hidden = session();
        hidden.archived = !archived_list;
        hidden.title = Some("hiddenonly".into());
        attach(&fixture, hidden, cx);
        attach(&fixture, visible.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.navigation.archived = archived_list;
                cx.notify();
            });
            window.render_frame(cx);
            window.press("ctrl-f", cx);
            window.input("hiddenonly", cx);
            window.press("enter", cx);
        })
        .expect("search keyboard input");
        cx.update(|cx| {
            assert_eq!(fixture.view.read(cx).selected.id, Some(visible.id));
        });
        assert!(
            fixture.commands.try_recv().is_err(),
            "an empty visible result set must not issue Watch for a hidden session"
        );
    }
}

#[gpui_kit::test]
fn a_catalog_started_before_creation_does_not_discard_new_drafts_or_queue(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(meta.clone().into()),
                },
                window,
                cx,
            );
            view.sessions.drafts.insert(
                meta.id,
                Draft {
                    text: "not yet sent".into(),
                    attachments: Vec::new(),
                },
            );
            view.sessions
                .outboxes
                .entry(meta.id)
                .or_default()
                .enqueue(Draft {
                    text: "queued".into(),
                    attachments: Vec::new(),
                });
            // This response was snapshotted before Create but delivered after its receipt.
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: Vec::new(),
                })),
                window,
                cx,
            );
            assert_eq!(view.sessions.drafts[&meta.id].text, "not yet sent");
            assert_eq!(
                view.sessions.outboxes[&meta.id]
                    .front()
                    .expect("queue")
                    .text,
                "queued"
            );
            // Subsequent serial directory observations can establish a real removal.
            for sessions in [vec![meta.clone()], Vec::new()] {
                view.update(
                    Update::Catalog(Ok(connection::Catalog {
                        agents: view.backend.agents.clone(),
                        sessions: sessions.into_iter().map(Into::into).collect(),
                    })),
                    window,
                    cx,
                );
            }
            assert!(!view.sessions.drafts.contains_key(&meta.id));
            assert!(!view.sessions.outboxes.contains_key(&meta.id));
        });
    })
    .expect("create and catalog interleaving");
}

#[gpui_kit::test]
fn delayed_archive_success_after_a_newer_restore_still_pauses_the_queue(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions
                .outboxes
                .entry(meta.id)
                .or_default()
                .enqueue(Draft {
                    text: "must wait for manual resume".into(),
                    attachments: Vec::new(),
                });
            view.set_archived(meta.id, true, cx);
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.backend.agents.clone(),
                    sessions: vec![archived(meta.clone(), false, 3).into()],
                })),
                window,
                cx,
            );
            assert!(
                !view.sessions.outboxes[&meta.id].paused(),
                "pending is not success"
            );
            view.update(
                Update::Archived {
                    session: meta.id,
                    result: Ok(archived(meta.clone(), true, 2).into()),
                },
                window,
                cx,
            );
            let current = view.current().expect("session");
            assert!(!current.archived);
            assert_eq!(current.revision, Cursor::from_raw(3));
            assert!(view.sessions.outboxes[&meta.id].paused());
            assert_eq!(view.sessions.outboxes[&meta.id].len(), 1);
        });
    })
    .expect("archive receipt after restore");
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::Archive { .. })
    ));
    assert!(
        fixture.commands.try_recv().is_err(),
        "restore must not send queued text"
    );
}

#[gpui_kit::test]
fn failed_archive_preserves_both_running_and_manually_paused_queue_state(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for paused in [false, true] {
                let outbox = view.sessions.outboxes.entry(meta.id).or_default();
                outbox.enqueue(Draft {
                    text: "queued".into(),
                    attachments: Vec::new(),
                });
                if paused {
                    outbox.pause();
                }
                view.set_archived(meta.id, true, cx);
                assert_eq!(view.sessions.outboxes[&meta.id].paused(), paused);
                view.update(
                    Update::Archived {
                        session: meta.id,
                        result: Err("rejected".into()),
                    },
                    window,
                    cx,
                );
                assert_eq!(view.sessions.outboxes[&meta.id].paused(), paused);
                assert!(!view.current().expect("meta").archived);
            }
        });
    })
    .expect("window");
}

#[gpui_kit::test]
fn inactive_catalog_archive_retains_attachments_and_does_not_resume_queue(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let file = tempfile::NamedTempFile::new().expect("attachment");
    std::fs::write(file.path(), "payload").expect("write");
    let attachment = crate::attachment::Attachment::load(file.path()).expect("load");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let draft = Draft {
                text: "retained".into(),
                attachments: vec![attachment.clone()],
            };
            view.sessions.drafts.insert(first.id, draft.clone());
            view.sessions
                .outboxes
                .entry(first.id)
                .or_default()
                .enqueue(draft);
            for (value, revision) in [(true, 2), (false, 3)] {
                view.update(
                    Update::Catalog(Ok(connection::Catalog {
                        agents: Vec::new(),
                        sessions: vec![
                            archived(first.clone(), value, revision).into(),
                            second.clone().into(),
                        ],
                    })),
                    window,
                    cx,
                );
                assert_eq!(view.selected.id, Some(second.id));
                assert!(view.sessions.outboxes[&first.id].paused());
                assert_eq!(
                    view.sessions.outboxes[&first.id]
                        .front()
                        .expect("head")
                        .attachments[0]
                        .id,
                    attachment.id
                );
                assert_eq!(
                    view.sessions.drafts[&first.id].attachments[0].id,
                    attachment.id
                );
            }
        });
    })
    .expect("window");
    assert!(fixture.commands.try_recv().is_err());
}

pub(super) fn archived(mut meta: SessionMeta, value: bool, revision: u64) -> SessionMeta {
    meta.archived = value;
    meta.revision = Cursor::from_raw(revision);
    meta
}

#[gpui_kit::test]
fn row_archive_restore_and_filter_keep_details_and_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover(format!("session-{}", first.id), cx);
        window.click(format!("archive-{}", first.id), cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(second.id));
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Archived {
                    session: first.id,
                    result: Ok(archived(first.clone(), true, 2).into()),
                },
                window,
                cx,
            );
            assert_eq!(view.visible_sessions().len(), 1);
            assert_eq!(view.sessions.entries.len(), 2);
        });
        window.render_frame(cx);
        assert!(window.try_find(format!("archive-{}", first.id)).is_none());
        window.click("archived-list", cx);
        window.render_frame(cx);
        window.hover(format!("session-{}", first.id), cx);
        assert!(window.find(format!("archive-{}", first.id)).visible());
        assert!(window.try_find(format!("archive-{}", second.id)).is_none());
        window.click(format!("archive-{}", first.id), cx);
        assert_eq!(fixture.view.read(cx).selected.id, Some(second.id));
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Archived {
                    session: first.id,
                    result: Ok(archived(first.clone(), false, 3).into()),
                },
                window,
                cx,
            );
            assert!(view.visible_sessions().is_empty());
        });
    })
    .expect("window");
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Archive { session, archived: true }) if session == first.id)
    );
    assert!(
        matches!(fixture.commands.try_recv(), Ok(Command::Archive { session, archived: false }) if session == first.id)
    );
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn pending_failure_and_background_catalog_preserve_edit_and_pause(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let outbox = view.sessions.outboxes.entry(meta.id).or_default();
            outbox.enqueue(Draft {
                text: "queued".into(),
                attachments: Vec::new(),
            });
            outbox.begin_edit(Draft {
                text: "displaced".into(),
                attachments: Vec::new(),
            });
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("edited head", window, cx));
            view.set_archived(meta.id, true, cx);
            view.set_archived(meta.id, true, cx);
            assert!(!view.can_send(cx));
            view.send_prompt(window, cx);
            view.update(
                Update::Archived {
                    session: meta.id,
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            assert_eq!(view.composer.input.read(cx).value(), "edited head");
            assert_eq!(
                view.sessions.outboxes[&meta.id].front().expect("head").text,
                "queued"
            );
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: Vec::new(),
                    sessions: vec![archived(meta.clone(), true, 4).into()],
                })),
                window,
                cx,
            );
            assert!(!view.can_send(cx));
            assert!(view.sessions.outboxes[&meta.id].paused());
            view.update(
                Update::Archived {
                    session: meta.id,
                    result: Ok(archived(meta.clone(), false, 3).into()),
                },
                window,
                cx,
            );
            assert!(view.current().expect("meta").archived);
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(5),
                        EventKind::ArchiveChanged { archived: false },
                    ),
                },
                window,
                cx,
            );
            assert!(view.sessions.outboxes[&meta.id].paused());
            assert_eq!(view.composer.input.read(cx).value(), "edited head");
            assert!(view.cancel_queue_edit(window, cx));
            assert_eq!(view.composer.input.read(cx).value(), "displaced");
        });
    })
    .expect("window");
    assert!(matches!(
        fixture.commands.try_recv(),
        Ok(Command::Archive { .. })
    ));
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn restored_launch_skips_archived_saved_and_latest_threads(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let old = archived(session(), true, 2);
    let live = session();
    let directory = tempfile::tempdir().expect("state directory");
    let mut store =
        workspace_navigation::restore::Store::open(directory.path().join("window.json"));
    store.save(Some(old.id)).expect("save");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries = vec![old, live.clone()];
            view.selected.creating = false;
            view.navigation.restore.on_connect = true;
            view.navigation.restore.store = Some(store);
            view.restore_thread(window, cx);
            assert_eq!(view.selected.id, Some(live.id));
        });
    })
    .expect("window");
}

#[gpui_kit::test]
fn archive_event_keeps_running_turn_cancel_and_restore_notice(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    meta.active_turn = Some(bas::model::TurnId::new());
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions
                .outboxes
                .entry(meta.id)
                .or_default()
                .enqueue(Draft {
                    text: "later".into(),
                    attachments: Vec::new(),
                });
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(4),
                        EventKind::ArchiveChanged { archived: true },
                    ),
                },
                window,
                cx,
            );
            assert!(view.working());
            assert_eq!(view.current().expect("meta").active_turn, meta.active_turn);
            assert!(view.sessions.outboxes[&meta.id].paused());
            assert!(!view.can_send(cx));
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: meta.clone().into(),
                    events: Vec::new(),
                },
                window,
                cx,
            );
            assert!(view.current().expect("metadata").archived);
        });
        window.render_frame(cx);
        assert!(
            window
                .find(format!("restore-archived-{}", meta.id))
                .visible()
        );
        window.click("stop-turn", cx);
    })
    .expect("window");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Cancel(id)) if id == meta.id));
}
