use super::*;

#[gpui_kit::test]
fn header_rows_align_and_icon_buttons_have_equal_spacing(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let sidebar = window.find("sidebar-header").bounds();
        let conversation = window.find("conversation-header-row").bounds();
        assert_eq!(sidebar.top(), conversation.top());
        assert_eq!(sidebar.size.height, conversation.size.height);
        let settings = window.find("open-settings").bounds();
        let workspace = window.find("add-workspace-button").bounds();
        let new_thread = window.find("new-session").bounds();
        for button in [settings, workspace, new_thread] {
            assert_eq!(
                button.size,
                size(px(layout::ICON_BUTTON_SIZE), px(layout::ICON_BUTTON_SIZE))
            );
            assert_eq!(button.top(), settings.top());
            assert!(button.top() >= sidebar.top() && button.bottom() <= sidebar.bottom());
        }
        assert_eq!(
            workspace.left() - settings.right(),
            new_thread.left() - workspace.right()
        );
        assert_eq!(window.find("sidebar-toggle").bounds().top(), settings.top());
        let menu = window.find("app-menu").bounds();
        assert!(menu.top() >= sidebar.top() && menu.bottom() <= sidebar.bottom());
        assert_eq!(menu.size, settings.size);
        assert!(window.try_find("project-logo-slot").is_none());
        fixture
            .view
            .update(cx, |view, cx| view.toggle_mode(window, cx));
        window.render_frame(cx);
        let compact = [
            "open-settings",
            "workspace-picker",
            "new-session",
            "toggle-mode",
        ]
        .map(|id| window.find(id).bounds());
        for button in compact {
            assert_eq!(button.size, settings.size);
            assert_eq!(button.top(), compact[0].top());
        }
        let gaps: Vec<_> = compact
            .windows(2)
            .map(|pair| pair[1].left() - pair[0].right())
            .collect();
        assert!(gaps.iter().all(|gap| *gap == gaps[0]));
    })
    .expect("aligned header layout");
}

#[gpui_kit::test]
fn archive_filter_is_outside_search_and_row_actions_stay_icon_sized(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let active = session();
    let mut archived = session();
    archived.archived = true;
    attach(&fixture, archived.clone(), cx);
    attach(&fixture, active.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let search = window.find("thread-search").bounds();
        let filter = window.find("archived-list").bounds();
        assert!(
            filter.left() >= search.right(),
            "filter must be a separate control outside the input"
        );
        // Fractional input height is snapped to device pixels independently.
        assert!((filter.center().y - search.center().y).abs() <= px(0.5));
        window.hover(format!("session-{}", active.id), cx);
        let action = window.find(format!("archive-{}", active.id)).bounds();
        assert_eq!(action.size.width, px(layout::ICON_BUTTON_SIZE));
        assert_eq!(
            window
                .find(format!("header-archive-{}", active.id))
                .bounds()
                .size,
            action.size
        );
        window.click("archived-list", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).archived_list);
        assert_eq!(fixture.view.read(cx).active, Some(active.id));
        let restore = window.find(format!("archive-{}", archived.id)).bounds();
        assert_eq!(restore.size, action.size);
        window.click("archived-list", cx);
        assert!(!fixture.view.read(cx).archived_list);
        assert!(
            fixture.commands.try_recv().is_err(),
            "filtering is not an RPC action"
        );
    })
    .expect("archive controls share the icon layout");
}
