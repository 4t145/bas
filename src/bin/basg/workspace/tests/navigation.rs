use super::*;
use gpui_kit::InputEvent;

#[gpui_kit::test]
fn sidebar_draft_title_double_click_renames_offline_and_preserves_discard(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.selected.creating = false;
            view.backend.connected = false;
            cx.notify();
        });
        window.click("draft-thread-title-text-0", cx);
        assert!(fixture.view.read(cx).renaming.edit.is_none());
        window.double_click("draft-thread-title-text-0", cx);
        let view = fixture.view.read(cx);
        assert!(view.renaming_draft());
        assert!(view.renaming.input.focus_handle(cx).is_focused(window));
        window.press("ctrl-a", cx);
        window.input("Local draft", cx);
        window.press("enter", cx);
    })
    .expect("edit local draft offline");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert_eq!(fixture.view.read(cx).draft_title(), "Local draft");
        for target in [
            "draft-thread-time-0",
            "draft-thread-agent-0",
            "draft-thread-title-0",
        ] {
            window.double_click(target, cx);
            assert!(fixture.view.read(cx).renaming.edit.is_none());
        }
        window.hover("draft-thread-0", cx);
        assert!(window.try_find("rename-draft-0").is_none());
        assert_eq!(
            window.find("draft-thread-actions-0").bounds().size.width,
            px(layout::ICON_BUTTON_SIZE)
        );
        window.click("discard-draft-0", cx);
        assert!(fixture.view.read(cx).navigation.drafts.metadata.is_empty());
        assert!(fixture.view.read(cx).renaming.edit.is_none());
    })
    .expect("local draft discard");
}

#[gpui_kit::test]
fn sidebar_draft_title_rejects_busy_and_stale_clicks(cx: &mut TestAppContext) {
    for blocked in ["creating", "pasting", "stale"] {
        let fixture = fixture(cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.selected.creating = false;
                cx.notify();
            });
            window.click("draft-thread-title-text-0", cx);
            let position = window.find("draft-thread-title-text-0").bounds().center();
            window.dispatch_event(
                gpui_kit::MouseDownEvent {
                    position,
                    button: gpui_kit::MouseButton::Left,
                    click_count: 2,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            fixture.view.update(cx, |view, _| match blocked {
                "creating" => view.selected.creating = true,
                "pasting" => view.composer.paste_pending = true,
                "stale" => view.composer.generation += 1,
                _ => unreachable!(),
            });
            window.dispatch_event(
                gpui_kit::MouseUpEvent {
                    position,
                    button: gpui_kit::MouseButton::Left,
                    click_count: 2,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            assert!(fixture.view.read(cx).renaming.edit.is_none(), "{blocked}");
        })
        .expect("blocked local draft rename");
    }
}

#[gpui_kit::test]
fn sidebar_title_clicks_select_then_rename_without_reselecting(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("keep this draft", window, cx);
            });
        });
        let title = format!("thread-title-text-{}", first.id);
        window.click(title.clone(), cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.selected.id, Some(first.id));
        assert!(view.renaming.edit.is_none());
        assert_eq!(view.sessions.drafts[&second.id].text, "keep this draft");
        let epoch = view.selected.epoch;
        window.double_click(title, cx);
        let view = fixture.view.read(cx);
        assert!(view.renaming_thread(first.id));
        assert_eq!(view.selected.epoch, epoch);
        assert!(view.renaming.input.focus_handle(cx).is_focused(window));
        assert!(window.try_find(format!("rename-{}", first.id)).is_none());
        let input = view.renaming.input.entity_id();
        window.double_click("rename-title-input", cx);
        assert_eq!(fixture.view.read(cx).renaming.input.entity_id(), input);
        window.press("escape", cx);
        assert!(fixture.view.read(cx).renaming.edit.is_none());
        for target in [
            format!("thread-time-{}", first.id),
            format!("thread-agent-{}", first.id),
            format!("thread-title-{}", first.id),
        ] {
            window.double_click(target, cx);
            assert!(fixture.view.read(cx).renaming.edit.is_none());
        }
        window.hover(format!("session-{}", first.id), cx);
        assert_eq!(
            window
                .find(format!("thread-actions-{}", first.id))
                .bounds()
                .size
                .width,
            px(layout::ICON_BUTTON_SIZE)
        );
        window.click(format!("archive-{}", first.id), cx);
        assert!(
            fixture
                .view
                .read(cx)
                .sessions
                .pending
                .archiving
                .contains(&first.id)
        );
        assert!(fixture.view.read(cx).renaming.edit.is_none());
    })
    .expect("sidebar pointer interactions");
}

#[gpui_kit::test]
fn sidebar_title_double_click_rejects_disabled_targets(cx: &mut TestAppContext) {
    for disabled in ["offline", "archiving", "deleting"] {
        let fixture = fixture(cx);
        let target = session();
        attach(&fixture, target.clone(), cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                match disabled {
                    "offline" => view.backend.connected = false,
                    "archiving" => {
                        view.sessions.pending.archiving.insert(target.id);
                    }
                    "deleting" => {
                        view.sessions.pending.deleting.insert(target.id);
                    }
                    _ => unreachable!(),
                }
                cx.notify();
            });
            window.double_click(format!("thread-title-text-{}", target.id), cx);
            assert!(fixture.view.read(cx).renaming.edit.is_none(), "{disabled}");
        })
        .expect("disabled title");
    }
}

#[gpui_kit::test]
fn sidebar_title_editor_commits_and_f2_keeps_header_behavior(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    attach(&fixture, target.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.press("f2", cx);
        assert!(matches!(
            fixture.view.read(cx).renaming.edit.as_ref(),
            Some(rename::Edit {
                location: rename::Location::Header,
                ..
            })
        ));
        window.press("escape", cx);
        window.double_click(format!("thread-title-text-{}", target.id), cx);
        window.press("ctrl-a", cx);
        window.input("New sidebar title", cx);
        window.press("enter", cx);
    })
    .expect("title commit and F2");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(fixture.view.read(cx).renaming.edit.is_none());
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(Command::Rename { session, title })
                if session == target.id && title == "New sidebar title"
        ));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("title commit receipt");
}

#[gpui_kit::test]
fn sidebar_title_context_menu_still_renames(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let target = session();
    attach(&fixture, target.clone(), cx);
    let popup = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.right_click(format!("thread-title-text-{}", target.id), cx);
            fixture
                .view
                .read(cx)
                .window
                .menu
                .as_ref()
                .expect("context menu")
                .window
        })
        .expect("open context menu");
    cx.update_window(popup.unwrap_or(fixture.window.into()), |_, window, cx| {
        for _ in 0..3 {
            window.press("down", cx);
        }
        window.press("enter", cx);
    })
    .expect("choose rename");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert!(view.renaming_thread(target.id));
        assert!(view.renaming.input.focus_handle(cx).is_focused(window));
    })
    .expect("menu rename focuses sidebar editor");
}

#[gpui_kit::test]
fn sidebar_title_stale_double_click_cannot_rename_removed_session(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let target = session();
    attach(&fixture, target.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.click(format!("thread-title-text-{}", target.id), cx);
        let position = window
            .find(format!("thread-title-text-{}", target.id))
            .bounds()
            .center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                position,
                button: gpui_kit::MouseButton::Left,
                click_count: 2,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        // Keep the painted listener alive while its catalog target becomes stale.
        fixture.view.update(cx, |view, _| {
            view.sessions.entries.retain(|entry| entry.id != target.id);
        });
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                position,
                button: gpui_kit::MouseButton::Left,
                click_count: 2,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        assert!(fixture.view.read(cx).renaming.edit.is_none());
    })
    .expect("stale title click");
}

#[gpui_kit::test]
fn switching_preserves_cached_history_but_detach_and_switch_clear_transient_controls(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.select(first.id, window, cx);
            view.composer.input.update(cx, |input, cx| {
                input.set_value("first composition", window, cx);
            });
            view.sessions
                .outboxes
                .entry(first.id)
                .or_default()
                .enqueue(Draft {
                    text: "background queued prompt".into(),
                    attachments: Vec::new(),
                });
            view.sessions.pending.configuring.insert(first.id);
            view.sessions.pending.loading_attachments.insert(first.id);
            view.sessions.pending.cancelling.insert(first.id);
            view.sessions.pending.archiving.insert(first.id);

            for detach in [false, true] {
                let current = view.selected.id.expect("selected thread");
                view.update(
                    history_window(
                        view.selected.epoch,
                        if current == first.id {
                            first.clone()
                        } else {
                            second.clone()
                        }
                        .into(),
                        Vec::new(),
                    ),
                    window,
                    cx,
                );
                view.update(
                    Update::Permissions {
                        epoch: view.selected.epoch,
                        pending: vec![approval::permission(current)],
                    },
                    window,
                    cx,
                );
                assert!(!view.selected.approval.panel.is_empty());
                view.selected.approval.error = Some("current approval failure".into());
                view.selected.authentication.required = Some("embedded".into());
                view.selected.error = Some("current failure".into());
                view.selected
                    .timeline
                    .expanded
                    .insert("current item".into());
                let old_epoch = view.selected.epoch;
                if detach {
                    view.new_thread(None, window, cx);
                    assert_eq!(view.selected.id, None);
                } else {
                    view.select(second.id, window, cx);
                    assert_eq!(view.selected.id, Some(second.id));
                }
                assert_eq!(view.selected.epoch, old_epoch + 1);
                if detach {
                    assert!(view.selected.timeline.conversation.is_none());
                } else {
                    assert_eq!(
                        view.selected
                            .timeline
                            .conversation
                            .as_ref()
                            .expect("cached second thread")
                            .session
                            .id,
                        second.id,
                    );
                }
                assert!(view.selected.approval.panel.is_empty());
                assert!(view.selected.approval.error.is_none());
                assert!(view.selected.authentication.required.is_none());
                assert!(view.selected.error.is_none());
                assert!(view.selected.timeline.expanded.is_empty());
                assert_eq!(view.sessions.drafts[&first.id].text, "first composition");
                assert_eq!(view.sessions.outboxes[&first.id].len(), 1);
                assert!(view.sessions.pending.configuring.contains(&first.id));
                assert!(
                    view.sessions
                        .pending
                        .loading_attachments
                        .contains(&first.id)
                );
                assert!(view.sessions.pending.cancelling.contains(&first.id));
                assert!(view.sessions.pending.archiving.contains(&first.id));
            }
            view.select(first.id, window, cx);
            assert_eq!(view.composer.input.read(cx).value(), "first composition");
        });
    })
    .expect("selection lifecycle preserves work owned by other sessions");
}
