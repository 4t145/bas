use super::*;

#[gpui_kit::test]
fn application_shortcuts_work_from_composer_and_settings(cx: &mut TestAppContext) {
    let (settings_key, quit_key) = if cfg!(target_os = "macos") {
        ("cmd-,", "cmd-q")
    } else {
        ("ctrl-,", "ctrl-q")
    };
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.resize_window(window, cx);
                view.focus_input(window, cx);
            });
            window.render_frame(cx);
            window.input("保留草稿", cx);
            window.press(settings_key, cx);
        })
        .expect("settings shortcut in composer");
        let settings = cx.update(|cx| crate::settings_window::current(cx).expect("settings"));
        cx.update_window(settings.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-search", cx);
            window.press(quit_key, cx);
        })
        .expect("quit shortcut in settings");
        let quit = cx
            .windows()
            .into_iter()
            .find(|handle| {
                handle.window_id() != fixture.window.window_id()
                    && handle.window_id() != settings.window_id()
            })
            .expect("shortcut opened quit confirmation");
        cx.update_window(quit, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("quit-window").visible());
            window.click("quit-cancel", cx);
        })
        .expect("cancel quit");
        cx.update_window(settings.into(), |_, window, cx| {
            window.dispatch_action(Box::new(crate::settings_window::CloseWindow), cx);
        })
        .expect("close settings");
        cx.update_window(fixture.window.into(), |_, window, cx| {
            let view = fixture.view.read(cx);
            assert_eq!(view.active, Some(meta.id));
            assert_eq!(view.composer.read(cx).value(), "保留草稿");
            window.remove_window();
        })
        .expect("cancel preserves workspace");
        while let Ok(command) = fixture.commands.try_recv() {
            assert!(matches!(command, Command::Foreground(_)));
        }
    }
}

fn open_menu(fixture: &Fixture, cx: &mut TestAppContext) -> gpui_kit::AnyWindowHandle {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window.click("app-menu", cx);
        fixture
            .view
            .read(cx)
            .menu
            .as_ref()
            .and_then(|menu| menu.window)
            .unwrap_or(fixture.window.into())
    })
    .expect("open application menu")
}

fn choose_item(fixture: &Fixture, index: usize, cx: &mut TestAppContext) {
    let popup = open_menu(fixture, cx);
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(index, cx);
    })
    .expect("choose menu item");
}

#[gpui_kit::test]
fn shortcut_dismisses_native_menu_before_quit_confirmation(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.compact = true;
            view.resize_window(window, cx);
        });
    })
    .expect("compact mode");
    let popup = open_menu(&fixture, cx);
    assert_ne!(popup.window_id(), fixture.window.window_id());
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        let key = if cfg!(target_os = "macos") {
            "cmd-q"
        } else {
            "ctrl-q"
        };
        window.press(key, cx);
    })
    .expect("quit from focused popup");
    cx.executor().run_until_parked();
    assert!(cx.update_window(popup, |_, _, _| ()).is_err());
    assert!(cx.update(|cx| fixture.view.read(cx).menu.is_none()));
    let quit = cx
        .windows()
        .into_iter()
        .find(|window| window.window_id() != fixture.window.window_id())
        .expect("quit confirmation");
    cx.update_window(quit, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("quit-window").visible());
        window.click("quit-cancel", cx);
    })
    .expect("cancel without stale menu");
}

#[gpui_kit::test]
fn application_menu_groups_labels_and_resolves_real_shortcut_hints(cx: &mut TestAppContext) {
    use gpui_kit::component::kbd::Kbd;
    for compact in [false, true] {
        let fixture = fixture(cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.resize_window(window, cx);
            });
        })
        .expect("set menu mode");
        let popup = open_menu(&fixture, cx);
        cx.update_window(popup, |_, window, cx| {
            window.render_frame(cx);
            let items = [
                (0, "关于 BASR"),
                (1, "查看发布记录"),
                (3, "设置"),
                (5, "退出 BASR"),
            ];
            let mut row_height = None;
            for (id, label) in items {
                let row = window.within("popup-menu").find(id);
                assert_eq!(row.role(), Some(gpui_kit::Role::MenuItem));
                assert_eq!(window.find(label).label(), Some(label));
                assert!(row.visible());
                assert_eq!(
                    *row_height.get_or_insert(row.bounds().size.height),
                    row.bounds().size.height
                );
            }
            for label in ["设置", "退出 BASR"] {
                let text = window.within(label).find("label").bounds();
                let shortcut = window.within(label).find("shortcut").bounds();
                assert_eq!(
                    text.size.height, shortcut.size.height,
                    "same menu text size"
                );
                assert_eq!(text.bottom(), shortcut.bottom(), "same menu baseline");
                assert!(text.right() <= shortcut.left());
            }
            for id in [2, 4] {
                let separator = window.within("popup-menu").find(id);
                assert!(separator.bounds().size.height < row_height.expect("item height"));
                assert!(
                    separator.bounds().top()
                        > window.within("popup-menu").find(id - 1).bounds().bottom()
                );
                assert!(
                    separator.bounds().bottom()
                        < window.within("popup-menu").find(id + 1).bounds().top()
                );
            }
            for action in [
                Box::new(crate::application::OpenSettings) as Box<dyn gpui_kit::Action>,
                Box::new(crate::application::Quit),
            ] {
                assert!(
                    Kbd::binding_for_action(action.as_ref(), None, window).is_some(),
                    "native and inline menus must resolve shortcuts from the real keymap"
                );
            }
            if compact {
                let content = window.find("popup-menu").bounds();
                assert!(content.bottom() <= window.bounds().size.height);
                let shadow = px(menus::SHADOW_PADDING);
                assert_eq!(content.left(), shadow);
                assert_eq!(window.bounds().size.width - content.right(), shadow);
                let bottom = window.bounds().size.height - content.bottom();
                assert!(bottom >= shadow && bottom <= shadow + px(1.));
            }
            window.press("escape", cx);
        })
        .expect("menu groups and hints");
        cx.update_window(fixture.window.into(), |_, window, _| window.remove_window())
            .expect("close fixture");
    }
}

#[gpui_kit::test]
fn application_menu_routes_actions_in_full_and_compact_modes_without_session_commands(
    cx: &mut TestAppContext,
) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.resize_window(window, cx);
                view.composer
                    .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            });
        })
        .expect("workspace mode");
        while fixture.commands.try_recv().is_ok() {}

        choose_item(&fixture, 3, cx);
        let settings = cx.update(|cx| {
            crate::settings_window::current(cx).expect("settings action opened a window")
        });
        cx.update_window(settings.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("settings-window").visible());
            window.dispatch_action(Box::new(crate::settings_window::CloseWindow), cx);
        })
        .expect("settings action uses its own window");

        choose_item(&fixture, 0, cx);
        let about = cx
            .windows()
            .into_iter()
            .find(|handle| handle.window_id() != fixture.window.window_id())
            .expect("about window");
        cx.update_window(about, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("about-window").visible());
            window.click("about-close", cx);
        })
        .expect("about action");

        cx.update(|cx| cx.open_url("about:blank"));
        choose_item(&fixture, 1, cx);
        assert_eq!(
            cx.opened_url().as_deref(),
            Some(crate::application::RELEASES_URL)
        );

        choose_item(&fixture, 5, cx);
        let quit = cx
            .windows()
            .into_iter()
            .find(|handle| handle.window_id() != fixture.window.window_id())
            .expect("quit confirmation");
        cx.update_window(quit, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("quit-window").visible());
            window.click("quit-cancel", cx);
        })
        .expect("quit is cancellable");
        cx.update(|cx| {
            let view = fixture.view.read(cx);
            assert_eq!(view.active, Some(meta.id));
            assert_eq!(view.composer.read(cx).value(), "keep draft");
        });
        while let Ok(command) = fixture.commands.try_recv() {
            assert!(
                matches!(command, Command::Foreground(_)),
                "app menu must not mutate sessions"
            );
        }
        cx.update_window(fixture.window.into(), |_, window, _| window.remove_window())
            .expect("close fixture");
    }
}
