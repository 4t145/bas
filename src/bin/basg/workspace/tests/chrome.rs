use super::*;

#[gpui_kit::test]
fn native_windows_share_workspace_control_and_icon_bounds(cx: &mut TestAppContext) {
    if cfg!(target_os = "macos") || cfg!(target_family = "wasm") {
        return;
    }
    let fixture = fixture(cx);
    let about = cx.update(|cx| crate::application::open_about(cx).expect("about window"));
    let settings = cx.update(|cx| crate::settings::window::open(cx).expect("settings window"));
    let mut baseline = None;
    for handle in [fixture.window, about, settings] {
        let bounds = cx
            .update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                ["window-minimize", "window-maximize", "window-close"].map(|id| {
                    let control = window.find(id).bounds().size;
                    let icon = window.find(format!("{id}-icon")).bounds().size;
                    assert_eq!(
                        control,
                        size(
                            px(crate::ui::COMPACT_CONTROL),
                            px(crate::ui::COMPACT_CONTROL)
                        )
                    );
                    assert_eq!(
                        icon,
                        size(px(crate::ui::SMALL_ICON), px(crate::ui::SMALL_ICON))
                    );
                    (control, icon)
                })
            })
            .expect("render native titlebar");
        if let Some(baseline) = baseline {
            assert_eq!(bounds, baseline);
        } else {
            baseline = Some(bounds);
        }
    }
}

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
        assert!(
            window.find("thread-search").bounds().top() >= sidebar.bottom() + px(8.),
            "thread search needs space below the toolbar"
        );
        let menu = window.find("app-menu").bounds();
        assert!(window.try_find("open-settings").is_none());
        let workspace = window.find("add-workspace-button").bounds();
        let new_thread = window.find("new-session").bounds();
        for button in [workspace, new_thread] {
            assert_eq!(
                button.size,
                size(px(layout::ICON_BUTTON_SIZE), px(layout::ICON_BUTTON_SIZE))
            );
            assert_eq!(button.top(), menu.top());
            assert_eq!(button.center().y, sidebar.center().y);
            assert!(button.top() >= sidebar.top() && button.bottom() <= sidebar.bottom());
        }
        assert_eq!(window.find("sidebar-toggle").bounds().top(), menu.top());
        assert!(menu.top() >= sidebar.top() && menu.bottom() <= sidebar.bottom());
        assert_eq!(menu.size, workspace.size);
        assert_eq!(menu.center().y, sidebar.center().y);
        assert!(window.try_find("project-logo-slot").is_none());
        let controls = [
            "toggle-mode",
            "window-minimize",
            "window-maximize",
            "window-close",
        ]
        .map(|id| window.find(id).bounds());
        for control in controls {
            assert_eq!(control.size, menu.size);
            assert_eq!(control.top(), menu.top());
            assert_eq!(control.center().y, conversation.center().y);
            assert!(control.right() <= conversation.right());
        }
        let control_gaps: Vec<_> = controls
            .windows(2)
            .map(|pair| pair[1].left() - pair[0].right())
            .collect();
        assert!(control_gaps.iter().all(|gap| *gap == control_gaps[0]));
        assert_eq!(new_thread.left() - workspace.right(), control_gaps[0]);
        window.click("sidebar-toggle", cx);
        window.render_frame(cx);
        assert!(
            window.find("app-menu").bounds().right()
                < window.find("sidebar-toggle").bounds().left()
        );
        assert_eq!(
            window.find("app-menu").bounds().top(),
            window.find("sidebar-toggle").bounds().top()
        );
        window.click("sidebar-toggle", cx);
        window.render_frame(cx);
        fixture
            .view
            .update(cx, |view, cx| view.toggle_mode(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("window-maximize").is_none());
        assert!(window.try_find("window-minimize").is_none());
        assert!(window.try_find("window-close").is_none());
        let compact = ["app-menu", "workspace-picker", "new-session", "toggle-mode"]
            .map(|id| window.find(id).bounds());
        for button in compact {
            assert_eq!(button.size, menu.size);
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
fn window_close_preserves_other_windows_and_cleans_up_workspace(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    let directory = tempfile::tempdir().expect("temporary restore store");
    let path = directory.path().join("window.json");
    let about = cx.update(|cx| crate::application::open_about(cx).expect("independent about"));
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, _| {
            view.navigation.restore.store =
                Some(workspace_navigation::restore::Store::open(path.clone()));
        });
        window.render_frame(cx);
        window.click("window-close", cx);
        let view = fixture.view.read(cx);
        assert!(view.window.menu.is_none());
        assert!(view.window.reply_preview.is_none());
    })
    .expect("close workspace");
    assert_eq!(
        workspace_navigation::restore::Store::open(path).thread(),
        Some(meta.id)
    );
    assert!(
        cx.update_window(fixture.window.into(), |_, _, _| ())
            .is_err()
    );
    cx.update_window(about.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("about-window").visible());
    })
    .expect("closing a workspace does not quit the app");
    while let Ok(command) = fixture.commands.try_recv() {
        assert!(matches!(command, Command::Foreground(_)));
    }
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
        assert!(
            window
                .try_find(format!("header-archive-{}", active.id))
                .is_none()
        );
        assert!(window.try_find("copy-session-id").is_none());
        assert!(window.try_find("refresh").is_none());
        window.click("archived-list", cx);
        window.render_frame(cx);
        assert_eq!(
            fixture.view.read(cx).navigation.view,
            workspace_navigation::View::Archived
        );
        assert_eq!(fixture.view.read(cx).selected.id, Some(active.id));
        let restore = window.find(format!("archive-{}", archived.id)).bounds();
        assert_eq!(restore.size, action.size);
        window.click("archived-list", cx);
        assert_eq!(
            fixture.view.read(cx).navigation.view,
            workspace_navigation::View::Workspaces
        );
        assert!(
            fixture.commands.try_recv().is_err(),
            "filtering is not an RPC action"
        );
    })
    .expect("archive controls share the icon layout");
}
