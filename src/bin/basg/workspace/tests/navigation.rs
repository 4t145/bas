use super::*;

#[gpui_kit::test]
fn switching_and_detaching_preserve_background_work_but_clear_current_state(
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
                    Update::Snapshot {
                        epoch: view.selected.epoch,
                        session: if current == first.id {
                            first.clone()
                        } else {
                            second.clone()
                        }
                        .into(),
                        events: Vec::new(),
                    },
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
                assert!(view.selected.timeline.conversation.is_none());
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

#[gpui_kit::test]
fn ctrl_tab_cycles_grouped_threads_and_preserves_drafts_in_both_modes(cx: &mut TestAppContext) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let mut first = session();
        first.cwd = Some("/workspace/a".into());
        let mut second = session();
        second.cwd = Some("/workspace/b".into());
        let mut third = session();
        third.cwd = first.cwd.clone();
        // Sidebar grouping is A: first,third; B: second, not the flat catalog order.
        for meta in [first.clone(), second.clone(), third.clone()] {
            attach(&fixture, meta, cx);
        }
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.window.compact = compact;
                view.select(first.id, window, cx);
                view.resize_window(window, cx);
            });
            window.render_frame(cx);
            window.input("first draft", cx);
        })
        .expect("prepare first thread");
        while fixture.commands.try_recv().is_ok() {}
        for (key, expected, input) in [
            ("ctrl-tab", third.id, Some("third draft")),
            ("ctrl-tab", second.id, Some("second draft")),
            ("ctrl-tab", first.id, None),
            ("ctrl-shift-tab", second.id, None),
        ] {
            cx.update_window(fixture.window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.press(key, cx);
                if let Some(input) = input {
                    window.input(input, cx);
                }
            })
            .expect("thread switch key");
            cx.update(|cx| {
                let view = fixture.view.read(cx);
                assert_eq!(view.selected.id, Some(expected));
                assert_eq!(view.window.compact, compact);
                if expected == first.id {
                    assert_eq!(view.composer.input.read(cx).value(), "first draft");
                }
                if expected == second.id {
                    assert_eq!(view.composer.input.read(cx).value(), "second draft");
                }
            });
        }
        let mut watches = Vec::new();
        while let Ok(command) = fixture.commands.try_recv() {
            match command {
                Command::Watch { session, .. } => watches.push(session),
                Command::Foreground(_) => {}
                _ => panic!("thread switching must not create, prompt, archive or cancel"),
            }
        }
        assert_eq!(watches, [third.id, second.id, first.id, second.id]);
    }
}

#[gpui_kit::test]
fn ctrl_tab_respects_search_and_archive_filters_and_empty_lists(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut first = session();
    first.title = Some("keep first".into());
    let mut second = session();
    second.title = Some("skip second".into());
    let mut archived = session();
    archived.title = Some("keep archived".into());
    archived.archived = true;
    for meta in [first.clone(), second.clone(), archived.clone()] {
        attach(&fixture, meta, cx);
    }
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigation.search.update(cx, |input, cx| {
                input.set_value("keep", window, cx);
                input.focus(window, cx);
            });
        });
        window.render_frame(cx);
        window.press("ctrl-tab", cx);
    })
    .expect("switch from search");
    cx.update(|cx| assert_eq!(fixture.view.read(cx).selected.id, Some(first.id)));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigation.archived = true;
            cx.notify();
        });
        window.render_frame(cx);
        window.press("ctrl-tab", cx);
    })
    .expect("archived list navigation");
    cx.update(|cx| assert_eq!(fixture.view.read(cx).selected.id, Some(archived.id)));
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigation
                .search
                .update(cx, |input, cx| input.set_value("no matches", window, cx));
        });
        window.render_frame(cx);
        window.press("ctrl-tab", cx);
        window.press("ctrl-shift-tab", cx);
    })
    .expect("empty filtered list");
    cx.update(|cx| assert_eq!(fixture.view.read(cx).selected.id, Some(archived.id)));
    assert!(fixture.commands.try_recv().is_err());
}
