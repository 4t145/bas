use super::*;

#[gpui_kit::test]
fn rows_status_changes_preserve_rail_title_and_hover_action(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let target = session();
    let id = target.id;
    attach(&fixture, target, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let title_id = format!("thread-title-{id}");
        let rail_id = format!("thread-rail-{id}");
        let title = window.find(title_id.clone()).bounds();
        let rail = window.find(rail_id.clone()).bounds();
        for (status, unread, dimensions) in [
            (SessionStatus::Starting, true, (12., 12.)),
            (SessionStatus::Working, true, (12., 12.)),
            (SessionStatus::Idle, true, (6., 6.)),
            (SessionStatus::Idle, false, (0., 0.)),
            (SessionStatus::Failed, false, (12., 12.)),
            (SessionStatus::Stopped, false, (8., 1.)),
        ] {
            fixture.view.update(cx, |view, cx| {
                let session = view
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == id)
                    .expect("row");
                session.status = status;
                session.last_reply = Cursor::from_raw(9);
                view.seen_replies.insert(id, if unread { 8 } else { 9 });
                cx.notify();
            });
            window.hover("thread-search", cx);
            window.render_frame(cx);
            assert_eq!(window.find(rail_id.clone()).bounds(), rail);
            assert_eq!(window.find(title_id.clone()).bounds(), title);
            let indicator = window.find(format!("thread-indicator-{id}")).bounds();
            assert_eq!(indicator.size, size(px(dimensions.0), px(dimensions.1)));
            if dimensions.0 > 0. {
                assert_eq!(indicator.center(), rail.center());
            }
            window.hover(format!("session-{id}"), cx);
            assert!(!window.find(format!("thread-status-{id}")).visible());
            assert!(window.find(format!("archive-{id}")).visible());
            assert_eq!(window.find(format!("archive-{id}")).bounds(), rail);
            assert_eq!(window.find(title_id.clone()).bounds(), title);
        }
    })
    .expect("status geometry remains stable");
}

#[gpui_kit::test]
fn rows_hover_swaps_equal_slots_and_group_plus_aligns(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    attach(&fixture, second.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover("thread-search", cx);
        let row_id = format!("session-{}", first.id);
        let title_id = format!("thread-title-{}", first.id);
        let status_id = format!("thread-status-{}", first.id);
        let archive_id = format!("archive-{}", first.id);
        let row = window.find(row_id.clone()).bounds();
        let title = window.find(title_id.clone()).bounds();
        let rail = window.find(format!("thread-rail-{}", first.id)).bounds();
        let plus = window.find("new-thread-0").bounds();
        assert_eq!(
            rail.size,
            size(px(layout::ICON_BUTTON_SIZE), px(layout::ICON_BUTTON_SIZE))
        );
        assert_eq!(plus.left(), rail.left());
        assert_eq!(plus.right(), rail.right());
        assert!(window.find(status_id.clone()).visible());
        assert!(!window.find(archive_id.clone()).visible());
        window.hover(row_id.clone(), cx);
        assert!(!window.find(status_id.clone()).visible());
        assert!(window.find(archive_id.clone()).visible());
        assert!(
            window
                .find(format!("thread-status-{}", second.id))
                .visible()
        );
        assert!(!window.find(format!("archive-{}", second.id)).visible());
        assert_eq!(window.find(archive_id).bounds(), rail);
        assert_eq!(window.find(row_id).bounds(), row);
        assert_eq!(window.find(title_id).bounds(), title);
        window.hover("thread-search", cx);
        assert!(window.find(status_id).visible());
    })
    .expect("row hover and aligned fixed rail");
}

#[gpui_kit::test]
fn rows_archive_filter_is_outside_input_on_same_line(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let input = window.find("thread-search").bounds();
        let filter = window.find("archived-list").bounds();
        assert!(filter.left() > input.right());
        assert!((filter.center().y - input.center().y).abs() <= px(0.5));
        window.click("archived-list", cx);
        assert!(fixture.view.read(cx).archived_list);
    })
    .expect("archive list control outside input border");
}

#[gpui_kit::test]
fn rows_archive_hover_action_does_not_select_target(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover(format!("session-{}", target.id), cx);
        window.click(format!("archive-{}", target.id), cx);
        assert_eq!(fixture.view.read(cx).active, Some(active.id));
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(Command::Archive { session, archived: true }) if session == target.id
        ));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("archive does not select or send");
}

// Context menus are built deferred by the component. Keep opening and confirming
// in separate updates so keyboard dispatch sees the rendered menu focus.
fn open_row_menu(
    fixture: &Fixture,
    id: SessionId,
    cx: &mut TestAppContext,
) -> gpui_kit::AnyWindowHandle {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.right_click(format!("session-{id}"), cx);
    })
    .expect("open row menu");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
    })
    .expect("render row menu");
    cx.update(|cx| {
        fixture
            .view
            .read(cx)
            .menu
            .as_ref()
            .and_then(|menu| menu.window)
            .unwrap_or(fixture.window.into())
    })
}

#[gpui_kit::test]
fn rows_context_pin_and_rename_target_without_selecting_on_open(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        assert_eq!(fixture.view.read(cx).active, Some(active.id));
        assert!(fixture.commands.try_recv().is_err());
        window.press("down", cx);
        window.press("enter", cx);
        assert!(fixture.view.read(cx).pinned.contains(&target.id));
        assert!(!fixture.view.read(cx).pinned.contains(&active.id));
        assert_eq!(fixture.view.read(cx).active, Some(active.id));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("pin correct row without RPC");
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(fixture.view.read(cx).active, Some(target.id));
        assert!(fixture.view.read(cx).editing_title);
        while let Ok(command) = fixture.commands.try_recv() {
            assert!(!matches!(command, Command::Prompt { .. }));
        }
    })
    .expect("rename correct row");
}

#[gpui_kit::test]
fn rows_context_archive_targets_clicked_row(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        for _ in 0..3 {
            window.press("down", cx);
        }
        window.press("enter", cx);
        assert_eq!(fixture.view.read(cx).active, Some(active.id));
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(Command::Archive { session, archived: true }) if session == target.id
        ));
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("menu archives clicked row only");
}

#[gpui_kit::test]
fn rows_context_delete_opens_confirmation_without_rpc(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let target = session();
    let active = session();
    attach(&fixture, target.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let menu = open_row_menu(&fixture, target.id, cx);
    cx.update_window(menu, |_, window, cx| {
        for _ in 0..4 {
            window.press("down", cx);
        }
        window.press("enter", cx);
        assert_eq!(fixture.view.read(cx).active, Some(active.id));
        assert!(
            fixture.commands.try_recv().is_err(),
            "delete requires confirmation"
        );
        assert!(!fixture.view.read(cx).delete_pending.contains(&target.id));
    })
    .expect("request confirmation only");
    cx.run_until_parked();
    let confirmation = cx
        .windows()
        .into_iter()
        .find(|window| window.window_id() != fixture.window.window_id())
        .expect("delete confirmation window");
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("confirmation-window").visible());
    })
    .expect("independent confirmation");
}
