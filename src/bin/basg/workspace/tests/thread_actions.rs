use super::*;

#[gpui_kit::test]
fn removal_batch_cannot_save_a_title_for_another_removed_thread(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let active = session();
    let editing = session();
    let retained = session();
    attach(&fixture, retained.clone(), cx);
    attach(&fixture, editing.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.drafts.insert(
                retained.id,
                Draft {
                    text: "retained draft".into(),
                    attachments: Vec::new(),
                },
            );
            view.begin_sidebar_rename(editing.id, window, cx);
            view.renaming
                .input
                .update(cx, |input, cx| input.set_value("must not save", window, cx));
            view.sessions_removed(&[active.id, editing.id], window, cx);
            assert_eq!(view.selected.id, Some(retained.id));
            assert_eq!(view.composer.input.read(cx).value(), "retained draft");
            assert!(view.renaming.edit.is_none());
            assert!(view.renaming.pending.is_empty());
            assert!(view.renaming.retries.is_empty());
            assert!(view.sessions.removed.contains(&active.id));
            assert!(view.sessions.removed.contains(&editing.id));
        });
        let commands: Vec<_> = std::iter::from_fn(|| fixture.commands.try_recv().ok()).collect();
        assert!(commands.iter().any(
            |command| matches!(command, Command::Watch { session, .. } if *session == retained.id)
        ));
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::Rename { .. }))
        );
    })
    .expect("batch removal before selection");
}

#[gpui_kit::test]
fn creation_receipt_after_catalog_removal_settles_without_reactivating(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let removed = session();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let epoch = view.selected.epoch;
            assert!(view.selected.creating);
            for sessions in [vec![removed.clone().into()], Vec::new()] {
                view.update(
                    Update::Catalog(Ok(connection::Catalog {
                        agents: view.backend.agents.clone(),
                        sessions,
                    })),
                    window,
                    cx,
                );
            }
            assert!(view.sessions.removed.contains(&removed.id));
            assert!(view.selected.creating);
            view.update(
                Update::Created {
                    epoch,
                    result: Ok(removed.clone().into()),
                },
                window,
                cx,
            );
            assert!(!view.selected.creating);
            assert!(view.selected.id.is_none());
            assert!(view.selected.error.is_some());
            assert!(view.sessions.entries.is_empty());
            assert!(!view.sessions.drafts.contains_key(&removed.id));
            assert!(fixture.commands.try_recv().is_err());
        });
    })
    .expect("deleted creation receipt");
}

#[gpui_kit::test]
fn catalog_removal_invalidates_selection_and_late_watch_updates(cx: &mut TestAppContext) {
    for keep_another_thread in [false, true] {
        let mut fixture = fixture(cx);
        let removed = session();
        let retained = session();
        if keep_another_thread {
            attach(&fixture, retained.clone(), cx);
        }
        attach(&fixture, removed.clone(), cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                let live = if keep_another_thread {
                    vec![removed.clone().into(), retained.clone().into()]
                } else {
                    vec![removed.clone().into()]
                };
                view.update(
                    Update::Catalog(Ok(connection::Catalog {
                        agents: view.backend.agents.clone(),
                        sessions: live,
                    })),
                    window,
                    cx,
                );
                view.composer
                    .input
                    .update(cx, |input, cx| input.set_value("outgoing", window, cx));
                view.sessions
                    .outboxes
                    .entry(removed.id)
                    .or_default()
                    .enqueue(Draft {
                        text: "must not send".into(),
                        attachments: Vec::new(),
                    });
                view.sessions
                    .outboxes
                    .get_mut(&removed.id)
                    .expect("queue")
                    .pause();
                view.sessions.pending.configuring.insert(removed.id);
                view.selected
                    .approval
                    .panel
                    .replace(vec![approval::permission(removed.id)]);
                let old_epoch = view.selected.epoch;
                let remaining = if keep_another_thread {
                    vec![retained.clone().into()]
                } else {
                    Vec::new()
                };
                view.update(
                    Update::Catalog(Ok(connection::Catalog {
                        agents: view.backend.agents.clone(),
                        sessions: remaining,
                    })),
                    window,
                    cx,
                );
                assert_ne!(view.selected.id, Some(removed.id));
                assert!(view.selected.epoch > old_epoch);
                assert!(view.current().is_none_or(|meta| meta.id != removed.id));
                assert!(view.selected.approval.panel.is_empty());
                assert!(!view.sessions.pending.configuring.contains(&removed.id));
                assert!(!view.sessions.drafts.contains_key(&removed.id));
                assert!(!view.sessions.outboxes.contains_key(&removed.id));
                let selected = view.selected.id;
                let epoch = view.selected.epoch;
                let composition = view.composer.input.read(cx).value();
                let creating = view.selected.creating;
                let turn = bas::model::TurnId::new();
                for incoming_epoch in [old_epoch, epoch] {
                    view.update(
                        Update::Snapshot {
                            epoch: incoming_epoch,
                            session: removed.clone().into(),
                            events: vec![Event::new(
                                removed.id,
                                Cursor::from_raw(1),
                                EventKind::TurnStarted { turn },
                            )],
                        },
                        window,
                        cx,
                    );
                    view.update(
                        Update::Event {
                            epoch: incoming_epoch,
                            event: Event::new(
                                removed.id,
                                Cursor::from_raw(2),
                                EventKind::TurnStarted { turn },
                            ),
                        },
                        window,
                        cx,
                    );
                }
                view.update(
                    Update::Stream {
                        epoch: old_epoch,
                        state: StreamState::Live,
                    },
                    window,
                    cx,
                );
                view.update(
                    Update::Permissions {
                        epoch: old_epoch,
                        pending: vec![approval::permission(removed.id)],
                    },
                    window,
                    cx,
                );
                view.update(
                    Update::Elicitations {
                        epoch: old_epoch,
                        pending: vec![elicitation::pending(removed.id, "url", false)],
                        completed_ids: Vec::new(),
                    },
                    window,
                    cx,
                );
                view.update(
                    Update::Configured {
                        session: removed.id,
                        result: Err("late".into()),
                    },
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
                view.select(removed.id, window, cx);
                assert_eq!(view.selected.id, selected);
                assert_eq!(view.selected.epoch, epoch);
                assert_eq!(view.selected.creating, creating);
                assert_eq!(view.composer.input.read(cx).value(), composition);
                assert!(view.current().is_none_or(|meta| meta.id != removed.id));
                assert!(view.selected.approval.panel.is_empty());
                assert!(view.selected.elicitations.is_empty());
                assert!(!view.sessions.prompted.contains(&removed.id));
                assert!(!view.sessions.drafts.contains_key(&removed.id));
                assert!(!view.sessions.errors.contains_key(&removed.id));
                assert!(!view.sessions.healthy(removed.id));
                while fixture.commands.try_recv().is_ok() {}
                let connected = view.backend.connected;
                view.set_archived(removed.id, true, cx);
                assert!(
                    fixture.commands.try_recv().is_err(),
                    "stale controls cannot target a removed session"
                );
                assert_eq!(
                    view.backend.connected, connected,
                    "local deletion is not a connection failure"
                );
                assert!(!view.sessions.pending.archiving.contains(&removed.id));
            });
        })
        .expect("catalog removal and delayed watch");
    }
}

#[gpui_kit::test]
fn deleting_current_thread_does_not_resurrect_its_outgoing_composition(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let removed = session();
    let retained = session();
    attach(&fixture, retained.clone(), cx);
    attach(&fixture, removed.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.drafts.insert(
                retained.id,
                Draft {
                    text: "retained composition".into(),
                    attachments: Vec::new(),
                },
            );
            view.composer.input.update(cx, |input, cx| {
                input.set_value("unsaved outgoing composition", window, cx);
            });
            view.sessions
                .outboxes
                .entry(removed.id)
                .or_default()
                .enqueue(Draft {
                    text: "removed queued prompt".into(),
                    attachments: Vec::new(),
                });
            view.update(
                Update::Deleted {
                    session: removed.id,
                    result: Ok(()),
                },
                window,
                cx,
            );
            assert_eq!(view.selected.id, Some(retained.id));
            assert_eq!(view.composer.input.read(cx).value(), "retained composition");
            assert!(!view.sessions.drafts.contains_key(&removed.id));
            assert!(!view.sessions.outboxes.contains_key(&removed.id));
            assert!(
                view.sessions
                    .entries
                    .iter()
                    .all(|meta| meta.id != removed.id)
            );
            view.new_thread(None, window, cx);
            assert!(!view.sessions.drafts.contains_key(&removed.id));
            assert_eq!(
                view.sessions.drafts[&retained.id].text,
                "retained composition"
            );
        });
    })
    .expect("deletion cleans up after saving the outgoing draft during selection");
}

#[gpui_kit::test]
fn discarding_drafts_rechecks_started_state_and_pending_work(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let draft = session();
    attach(&fixture, draft.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(!view.sessions.is_draft(draft.id));
            view.sessions.unstarted.insert(draft.id);
            assert!(view.sessions.is_draft(draft.id));
            view.sessions
                .outboxes
                .entry(draft.id)
                .or_default()
                .enqueue(Draft {
                    text: "pending prompt".into(),
                    attachments: Vec::new(),
                });
            view.discard_session_draft(draft.id, window, cx);
            assert!(!view.sessions.pending.deleting.contains(&draft.id));
            assert!(fixture.commands.try_recv().is_err());

            view.sessions.outboxes.remove(&draft.id);
            view.record_user_prompt(draft.id);
            view.discard_session_draft(draft.id, window, cx);
            assert!(!view.sessions.pending.deleting.contains(&draft.id));
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
            view.sessions.unstarted.insert(draft.id);
            view.composer.input.update(cx, |input, cx| {
                input.set_value("retained draft", window, cx);
            });
            view.discard_session_draft(draft.id, window, cx);
            view.discard_session_draft(draft.id, window, cx);
            assert!(view.sessions.pending.deleting.contains(&draft.id));
            assert_eq!(view.composer.input.read(cx).value(), "retained draft");
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
            assert!(!view.sessions.pending.deleting.contains(&draft.id));
            assert!(view.sessions.entries.iter().any(|meta| meta.id == draft.id));
            assert_eq!(view.composer.input.read(cx).value(), "retained draft");
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
        assert!(!view.sessions.pending.deleting.contains(&meta.id));
        assert!(
            view.sessions
                .entries
                .iter()
                .any(|session| session.id == meta.id)
        );
        assert!(
            view.sessions
                .errors
                .get(&meta.id)
                .is_some_and(|error| error.contains("请重新打开 BasG"))
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
                    agents: view.backend.agents.clone(),
                    sessions: vec![removed.clone().into(), retained.clone().into()],
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
                    agents: view.backend.agents.clone(),
                    sessions: vec![removed.clone().into(), retained.clone().into()],
                })),
                window,
                cx,
            );
            view.update(
                Update::Archived {
                    session: removed.id,
                    result: Ok(removed.clone().into()),
                },
                window,
                cx,
            );
            view.update(
                Update::Created {
                    epoch: view.selected.epoch,
                    result: Ok(removed.clone().into()),
                },
                window,
                cx,
            );
            assert!(
                view.sessions
                    .entries
                    .iter()
                    .all(|meta| meta.id != removed.id)
            );
            assert!(!view.sessions.drafts.contains_key(&removed.id));
            assert!(!view.sessions.outboxes.contains_key(&removed.id));
            assert_eq!(view.selected.id, Some(retained.id));
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
                        epoch: view.selected.epoch,
                        event,
                    },
                    window,
                    cx,
                );
            }
            let turn = bas::model::TurnId::new();
            for (cursor, kind) in [
                (4, EventKind::TurnStarted { turn }),
                (
                    5,
                    EventKind::TurnFinished {
                        turn,
                        result: Ok(bas::acp::PromptResponse::new(
                            bas::acp::StopReason::Cancelled,
                        )),
                    },
                ),
            ] {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: Event::new(meta.id, Cursor::from_raw(cursor), kind),
                    },
                    window,
                    cx,
                );
            }
            assert!(
                view.selected
                    .timeline
                    .conversation
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
        assert_eq!(
            fixture.view.read(cx).sessions.seen_replies.get(&meta.id),
            Some(&2)
        );
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
                view.selected
                    .timeline
                    .conversation
                    .as_ref()
                    .expect("empty replay")
                    .last_reply
                    .is_none()
            );
            view.mark_thread_read(meta.id, cx);
            assert_eq!(view.sessions.seen_replies.get(&meta.id), Some(&100));
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
            let conversation = view
                .selected
                .timeline
                .conversation
                .as_mut()
                .expect("attached conversation");
            for event in completed_events(meta.id, "replayed reply") {
                conversation.apply(event);
            }
            assert_eq!(conversation.session.last_reply, Cursor::START);
            assert_eq!(conversation.last_reply.expect("replayed content").cursor, 2);
            view.mark_thread_read(meta.id, cx);
            assert_eq!(view.sessions.seen_replies.get(&meta.id), Some(&2));
            view.sessions.seen_replies.insert(meta.id, 10);
            view.mark_thread_read(meta.id, cx);
            assert_eq!(view.sessions.seen_replies.get(&meta.id), Some(&10));
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
            view.navigation.restore.store =
                Some(workspace_navigation::restore::Store::open(path.clone()));
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
                workspace_navigation::restore::Store::open(path.clone())
                    .pinned()
                    .contains(&third.id)
            );
            view.toggle_pinned(third.id, cx);
            assert!(
                !workspace_navigation::restore::Store::open(path.clone())
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
            view.sessions
                .outboxes
                .entry(meta.id)
                .or_default()
                .enqueue(Draft {
                    text: "queued".into(),
                    attachments: Vec::new(),
                });
            view.set_archived(meta.id, true, cx);
            view.update(
                Update::Archived {
                    session: meta.id,
                    result: Err("当前 bas 不支持归档".into()),
                },
                window,
                cx,
            );
            assert!(!view.sessions.pending.archiving.contains(&meta.id));
            assert!(!view.current().expect("session").archived);
            assert_eq!(
                view.sessions.outboxes[&meta.id]
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
            assert!(view.sessions.entries.iter().any(|item| item.id == meta.id));
            assert!(!view.sessions.pending.deleting.contains(&meta.id));
        });
    })
    .expect("failed delete preserves session");
}
