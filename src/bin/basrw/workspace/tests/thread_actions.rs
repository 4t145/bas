use super::*;

#[gpui_kit::test]
fn discarding_drafts_rechecks_started_state_and_pending_work(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let draft = session();
    attach(&fixture, draft.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(!view.session_is_draft(draft.id));
            view.unstarted_sessions.insert(draft.id);
            assert!(view.session_is_draft(draft.id));
            view.outboxes.entry(draft.id).or_default().enqueue(Draft {
                text: "pending prompt".into(),
                attachments: Vec::new(),
            });
            view.discard_session_draft(draft.id, window, cx);
            assert!(!view.delete_pending.contains(&draft.id));
            assert!(fixture.commands.try_recv().is_err());

            view.outboxes.remove(&draft.id);
            view.record_user_prompt(draft.id);
            view.discard_session_draft(draft.id, window, cx);
            assert!(!view.delete_pending.contains(&draft.id));
            assert!(fixture.commands.try_recv().is_err());
        });
    })
    .expect("a stale discard action cannot delete a started or pending thread");
}

#[gpui_kit::test]
fn failed_draft_discard_retains_text_until_delete_succeeds(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let draft = session();
    attach(&fixture, draft.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.unstarted_sessions.insert(draft.id);
            view.composer.update(cx, |input, cx| {
                input.set_value("retained draft", window, cx);
            });
            view.discard_session_draft(draft.id, window, cx);
            view.discard_session_draft(draft.id, window, cx);
            assert!(view.delete_pending.contains(&draft.id));
            assert_eq!(view.composer.read(cx).value(), "retained draft");
            assert!(
                matches!(fixture.commands.try_recv(), Ok(Command::Delete(id)) if id == draft.id)
            );
            assert!(fixture.commands.try_recv().is_err());
            view.update(
                Update::Deleted {
                    session: draft.id,
                    result: Err("unavailable".into()),
                },
                window,
                cx,
            );
            assert!(!view.delete_pending.contains(&draft.id));
            assert!(view.sessions.iter().any(|meta| meta.id == draft.id));
            assert_eq!(view.composer.read(cx).value(), "retained draft");
        });
    })
    .expect("failed deletion leaves the draft available for retry");
}
#[gpui_kit::test]
fn closed_connection_does_not_leave_deletion_pending(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    fixture.commands.close();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.request_delete(meta.id, window, cx));
    })
    .expect("delete confirmation");
    let confirmation = cx
        .windows()
        .into_iter()
        .find(|handle| handle.window_id() != fixture.window.window_id())
        .expect("confirmation");
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        window.click("message-confirm", cx);
    })
    .expect("confirm with stopped connection");
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert!(!view.delete_pending.contains(&meta.id));
        assert!(view.sessions.iter().any(|session| session.id == meta.id));
        assert!(
            view.error
                .as_deref()
                .is_some_and(|error| error.contains("请重新打开 BASR"))
        );
    });
}

#[gpui_kit::test]
fn confirmed_deletion_cannot_be_undone_by_old_catalogs_or_receipts(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let removed = session();
    let retained = session();
    attach(&fixture, removed.clone(), cx);
    attach(&fixture, retained.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.agents.clone(),
                    sessions: vec![removed.clone(), retained.clone()],
                })),
                window,
                cx,
            );
            view.update(
                Update::Deleted {
                    session: removed.id,
                    result: Ok(()),
                },
                window,
                cx,
            );
            view.update(
                Update::Catalog(Ok(connection::Catalog {
                    agents: view.agents.clone(),
                    sessions: vec![removed.clone(), retained.clone()],
                })),
                window,
                cx,
            );
            view.update(
                Update::Archived {
                    session: removed.id,
                    result: Ok(removed.clone()),
                },
                window,
                cx,
            );
            view.update(
                Update::Created {
                    epoch: view.epoch,
                    result: Ok(removed.clone()),
                },
                window,
                cx,
            );
            assert!(view.sessions.iter().all(|meta| meta.id != removed.id));
            assert!(!view.drafts.contains_key(&removed.id));
            assert!(!view.outboxes.contains_key(&removed.id));
            assert_eq!(view.active, Some(retained.id));
        });
    })
    .expect("late catalog and receipt sequence");
}

#[gpui_kit::test]
fn a_turn_without_new_reply_does_not_prevent_reading_older_visible_content(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for event in completed_events(meta.id, "previous reply") {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event,
                    },
                    window,
                    cx,
                );
            }
            let turn = basr::model::TurnId::new();
            for (cursor, kind) in [
                (4, EventKind::TurnStarted { turn }),
                (
                    5,
                    EventKind::TurnFinished {
                        turn,
                        result: Ok(basr::acp::PromptResponse::new(
                            basr::acp::StopReason::Cancelled,
                        )),
                    },
                ),
            ] {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(meta.id, Cursor::from_raw(cursor), kind),
                    },
                    window,
                    cx,
                );
            }
            assert!(
                view.conversation
                    .as_ref()
                    .expect("conversation")
                    .completed_reply
                    .is_none()
            );
        });
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("view older agent content");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).seen_replies.get(&meta.id), Some(&2));
    })
    .expect("visible content clears unread");
}

#[gpui_kit::test]
fn unread_evicted_history_can_be_explicitly_acknowledged(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.revision = Cursor::from_raw(150);
    meta.last_reply = Cursor::from_raw(100);
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, _, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(
                view.conversation
                    .as_ref()
                    .expect("empty replay")
                    .last_reply
                    .is_none()
            );
            view.mark_thread_read(meta.id, cx);
            assert_eq!(view.seen_replies.get(&meta.id), Some(&100));
        });
        assert!(
            fixture.commands.try_recv().is_err(),
            "read acknowledgement is local"
        );
    })
    .expect("acknowledge unavailable history");
}

#[gpui_kit::test]
fn read_acknowledgement_uses_replay_when_old_metadata_has_no_reply_cursor(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.revision = Cursor::from_raw(150);
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, _, cx| {
        fixture.view.update(cx, |view, cx| {
            let conversation = view.conversation.as_mut().expect("attached conversation");
            for event in completed_events(meta.id, "replayed reply") {
                conversation.apply(event);
            }
            assert_eq!(conversation.session.last_reply, Cursor::START);
            assert_eq!(conversation.last_reply.expect("replayed content").cursor, 2);
            view.mark_thread_read(meta.id, cx);
            assert_eq!(view.seen_replies.get(&meta.id), Some(&2));
            view.seen_replies.insert(meta.id, 10);
            view.mark_thread_read(meta.id, cx);
            assert_eq!(view.seen_replies.get(&meta.id), Some(&10));
        });
    })
    .expect("acknowledge content from an older daemon");
}

#[gpui_kit::test]
fn pinning_reorders_only_its_workspace_and_survives_reopen(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut first = session();
    first.cwd = Some("/a".into());
    let mut second = session();
    second.cwd = Some("/b".into());
    let mut third = session();
    third.cwd = Some("/a".into());
    for meta in [first.clone(), second.clone(), third.clone()] {
        attach(&fixture, meta, cx);
    }
    let dir = tempfile::tempdir().expect("state");
    let path = dir.path().join("window.json");
    cx.update_window(fixture.window.into(), |_, _, cx| {
        fixture.view.update(cx, |view, cx| {
            view.restore_store = Some(restore::Store::open(path.clone()));
            view.toggle_pinned(third.id, cx);
            let sessions = view.visible_sessions();
            let groups = view.grouped_threads(&sessions, "");
            assert_eq!(
                groups[0]
                    .threads
                    .iter()
                    .map(|meta| meta.id)
                    .collect::<Vec<_>>(),
                [third.id, first.id]
            );
            assert_eq!(groups[1].threads[0].id, second.id);
            assert!(
                restore::Store::open(path.clone())
                    .pinned()
                    .contains(&third.id)
            );
            view.toggle_pinned(third.id, cx);
            assert!(
                !restore::Store::open(path.clone())
                    .pinned()
                    .contains(&third.id)
            );
        });
    })
    .expect("pin actions");
}

#[gpui_kit::test]
fn archive_failure_opens_visible_message_and_preserves_queue(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.outboxes.entry(meta.id).or_default().enqueue(Draft {
                text: "queued".into(),
                attachments: Vec::new(),
            });
            view.set_archived(meta.id, true, cx);
            view.update(
                Update::Archived {
                    session: meta.id,
                    result: Err("当前 basrd 不支持归档".into()),
                },
                window,
                cx,
            );
            assert!(!view.archive_pending.contains(&meta.id));
            assert!(!view.current().expect("session").archived);
            assert_eq!(
                view.outboxes[&meta.id]
                    .front()
                    .expect("queue retained")
                    .text,
                "queued"
            );
        });
    })
    .expect("archive error");
    let windows = cx.windows();
    assert_eq!(windows.len(), 2);
    let error = windows
        .into_iter()
        .find(|handle| handle.window_id() != fixture.window.window_id())
        .expect("error window");
    cx.update_window(error, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("message-window").visible());
    })
    .expect("visible feedback");
}

#[gpui_kit::test]
fn permanent_delete_requires_confirmation_and_is_separate_from_archive(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.request_delete(meta.id, window, cx));
    })
    .expect("request delete");
    assert!(
        fixture.commands.try_recv().is_err(),
        "opening confirmation sends no delete"
    );
    let confirmation = cx
        .windows()
        .into_iter()
        .find(|handle| handle.window_id() != fixture.window.window_id())
        .expect("confirmation");
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        window.click("message-close", cx);
    })
    .expect("cancel deletion");
    assert!(fixture.commands.try_recv().is_err());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.request_delete(meta.id, window, cx));
    })
    .expect("request again");
    let confirmation = cx
        .windows()
        .into_iter()
        .find(|handle| handle.window_id() != fixture.window.window_id())
        .expect("confirmation");
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        window.click("message-confirm", cx);
    })
    .expect("confirm deletion");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Delete(id)) if id == meta.id));
    assert!(fixture.commands.try_recv().is_err());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Deleted {
                    session: meta.id,
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            assert!(view.sessions.iter().any(|item| item.id == meta.id));
            assert!(!view.delete_pending.contains(&meta.id));
        });
    })
    .expect("failed delete preserves session");
}
