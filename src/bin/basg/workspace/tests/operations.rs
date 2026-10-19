use super::*;

#[gpui_kit::test]
fn stale_controls_cannot_retarget_approvals_or_queue_compositions(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    let old_epoch = cx.update(|cx| fixture.view.read(cx).selected.epoch);
    attach(&fixture, second.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| input.set_value("B composition", window, cx));
            view.selected.approval.panel.replace(vec![approval::permission(second.id)]);
            let outcome = bas::acp::RequestPermissionOutcome::Cancelled;
            view.respond_permission(first.id, old_epoch, "live-request".into(), outcome.clone(), cx);
            view.respond_permission(second.id, old_epoch, "live-request".into(), outcome.clone(), cx);
            view.respond_permission(second.id, view.selected.epoch, "absent".into(), outcome.clone(), cx);
            assert!(fixture.commands.try_recv().is_err());
            view.respond_permission(second.id, view.selected.epoch, "live-request".into(), outcome.clone(), cx);
            assert!(matches!(fixture.commands.try_recv(), Ok(Command::RespondPermission { session, .. }) if session == second.id));
            view.respond_permission(second.id, view.selected.epoch, "live-request".into(), outcome, cx);
            assert!(fixture.commands.try_recv().is_err(), "same pending approval cannot be sent twice");

            let queued = Draft { text: "A queued".into(), attachments: Vec::new() };
            view.sessions.outboxes.entry(first.id).or_default().enqueue(queued);
            view.edit_queue_draft(first.id, 0, window, cx);
            assert!(view.sessions.outboxes[&first.id].editing().is_none());
            view.sessions.outboxes.get_mut(&first.id).expect("A queue").begin_edit(Draft {
                text: "A composition".into(), attachments: Vec::new(),
            });
            view.withdraw_queue_draft(first.id, 0, window, cx);
            assert_eq!(view.sessions.outboxes[&first.id].len(), 1);
            assert_eq!(view.composer.input.read(cx).value(), "B composition");
        });
    }).expect("stale controls window");
}

#[gpui_kit::test]
fn closed_command_channel_finishes_pending_operations_without_losing_drafts(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    fixture.commands.close();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            view.sessions.pending.configuring.insert(meta.id);
            assert!(!view.command(
                Command::SetConfig(bas::acp::SetSessionConfigOptionRequest::new(
                    meta.id.to_string(),
                    "mode",
                    "fast",
                )),
                cx
            ));
            assert!(!view.sessions.pending.configuring.contains(&meta.id));
            assert!(view.sessions.errors.contains_key(&meta.id));

            view.sessions
                .outboxes
                .entry(meta.id)
                .or_default()
                .enqueue_immediate(
                    Draft {
                        text: "queued".into(),
                        attachments: Vec::new(),
                    },
                    true,
                );
            view.sessions.pending.cancelling.insert(meta.id);
            assert!(!view.command(Command::Cancel(meta.id), cx));
            assert!(!view.sessions.pending.cancelling.contains(&meta.id));
            let outbox = &view.sessions.outboxes[&meta.id];
            assert!(outbox.paused());
            assert_eq!(outbox.front().expect("retained queue").text, "queued");

            view.renaming.pending.insert(meta.id, " raw title ".into());
            assert!(!view.command(
                Command::Rename {
                    session: meta.id,
                    title: "raw title".into()
                },
                cx
            ));
            assert!(!view.renaming.pending.contains_key(&meta.id));
            assert_eq!(view.renaming.retries[&meta.id], " raw title ");
            view.sessions.pending.archiving.insert(meta.id);
            assert!(!view.command(
                Command::Archive {
                    session: meta.id,
                    archived: true
                },
                cx
            ));
            assert!(!view.sessions.pending.archiving.contains(&meta.id));
            view.sessions.pending.deleting.insert(meta.id);
            assert!(!view.command(Command::Delete(meta.id), cx));
            assert!(!view.sessions.pending.deleting.contains(&meta.id));
            assert!(!view.sessions.removed.contains(&meta.id));
            assert_eq!(view.composer.input.read(cx).value(), "keep draft");
        });
    })
    .expect("operation failure window");
}
