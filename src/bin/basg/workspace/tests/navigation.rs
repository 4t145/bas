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
