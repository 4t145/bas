use super::*;

pub(super) fn permission(session: SessionId) -> bas::model::PendingPermission {
    bas::model::PendingPermission {
        request_id: "live-request".into(),
        request: bas::acp::RequestPermissionRequest::new(
            session.to_string(),
            bas::acp::ToolCallUpdate::new(
                "write-file",
                bas::acp::ToolCallUpdateFields::new().title("写入配置"),
            ),
            vec![bas::acp::PermissionOption::new(
                "allow-once",
                "允许一次",
                bas::acp::PermissionOptionKind::AllowOnce,
            )],
        ),
    }
}

#[gpui_kit::test]
fn late_approval_snapshots_cannot_appear_in_another_thread(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    let old_epoch = cx.update(|cx| fixture.view.read(cx).selected.epoch);
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
            assert!(view.selected.approval.panel.is_empty());
        });
        window.render_frame(cx);
        assert!(window.try_find("approval-panel").is_none());
    })
    .expect("approval epoch isolation");
}
