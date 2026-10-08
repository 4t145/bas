use super::*;

fn permission(session: SessionId) -> basr::model::PendingPermission {
    basr::model::PendingPermission {
        request_id: "live-request".into(),
        request: basr::acp::RequestPermissionRequest::new(
            session.to_string(),
            basr::acp::ToolCallUpdate::new(
                "write-file",
                basr::acp::ToolCallUpdateFields::new().title("写入配置"),
            ),
            vec![basr::acp::PermissionOption::new(
                "allow-once",
                "允许一次",
                basr::acp::PermissionOptionKind::AllowOnce,
            )],
        ),
    }
}

#[gpui_kit::test]
fn compact_approval_is_visible_and_submits_the_agent_choice_once(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.toggle_mode(window, cx);
            assert!(!view.output_open);
            view.update(Update::Permissions {
                epoch: view.epoch, pending: vec![permission(meta.id)],
            }, window, cx);
            assert!(view.output_open);
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        let choice = "permission-live-request-allow-once";
        assert!(window.find("approval-panel").visible());
        assert!(window.find(choice).visible());
        let panel = window.find("approval-panel").bounds();
        assert!(panel.bottom() <= window.find("composer-editor").bounds().top());
        window.click(choice, cx);
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::RespondPermission {
            session, request_id, outcome: basr::acp::RequestPermissionOutcome::Selected(choice),
        }) if session == meta.id && request_id == "live-request" && choice.option_id.to_string() == "allow-once"));
        window.click(choice, cx);
        assert!(fixture.commands.try_recv().is_err());
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Permissions { epoch: view.epoch, pending: Vec::new() }, window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("approval-panel").is_none());
    }).expect("compact approval window");
}

#[gpui_kit::test]
fn late_approval_snapshots_cannot_appear_in_another_thread(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    let old_epoch = cx.update(|cx| fixture.view.read(cx).epoch);
    attach(&fixture, second, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Permissions {
                    epoch: old_epoch,
                    pending: vec![permission(first.id)],
                },
                window,
                cx,
            );
            assert!(view.approvals.is_empty());
        });
        window.render_frame(cx);
        assert!(window.try_find("approval-panel").is_none());
    })
    .expect("approval epoch isolation");
}

#[gpui_kit::test]
fn unavailable_approval_blocks_new_sends_and_preserves_a_paused_queue(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .update(cx, |input, cx| input.set_value("新草稿", window, cx));
            view.outboxes.entry(meta.id).or_default().enqueue(Draft {
                text: "待发送的旧消息".into(),
                attachments: Vec::new(),
            });
            view.update(
                Update::PermissionUnavailable {
                    epoch: view.epoch,
                    message: "审批连接已断开".into(),
                },
                window,
                cx,
            );
            assert!(!view.can_send(cx));
            view.send_prompt(window, cx);
            assert_eq!(view.composer.read(cx).value(), "新草稿");
            assert!(view.outboxes[&meta.id].paused());
            assert_eq!(view.outboxes[&meta.id].len(), 1);
            view.update(
                Update::Permissions {
                    epoch: view.epoch,
                    pending: Vec::new(),
                },
                window,
                cx,
            );
            assert!(view.can_send(cx));
            assert!(view.outboxes[&meta.id].paused());
        });
        assert!(
            fixture.commands.try_recv().is_err(),
            "recovering approval does not send a paused queue"
        );
    })
    .expect("approval recovery preserves input and queue");
}
