use super::*;
use crate::settings_window;

fn settings_window(cx: &mut TestAppContext) -> WindowHandle<Root> {
    cx.update(|cx| settings_window::current(cx).expect("settings window is registered"))
}

fn close_settings(cx: &mut TestAppContext) {
    let handle = settings_window(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(settings_window::CloseWindow), cx);
    })
    .expect("close the settings window");
}

#[gpui_kit::test]
fn enter_preference_saved_in_settings_updates_workspace_without_taking_focus(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("尚未发送的草稿", window, cx)
            });
            view.begin_rename(window, cx);
        });
        window.render_frame(cx);
    })
    .expect("show the title editor before opening settings");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.is_window_active());
        assert!(
            fixture
                .view
                .read(cx)
                .rename
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        fixture
            .view
            .update(cx, |view, cx| view.open_settings(window, cx));
    })
    .expect("open independent settings window");
    let settings = settings_window(cx);
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        let editor = theme::Editor::current(cx);
        let mut document = editor.load().expect("current settings");
        let mut preferences = document.preferences.clone();
        preferences.composer_enter_behavior = composer::Mode::Newline;
        editor
            .save(&mut document, preferences, window, cx)
            .expect("persist Enter behavior");
    })
    .expect("save from settings");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(!window.is_window_active());
    })
    .expect("draw the inactive workspace to dispatch title Blur");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.composer_mode, composer::Mode::Newline);
        assert_eq!(view.composer.read(cx).value(), "尚未发送的草稿");
        assert!(view.title_edit.is_none());
        assert!(!view.composer.read(cx).focus_handle(cx).is_focused(window));
        assert!(!window.is_window_active());
    })
    .expect("workspace updates without becoming active");
    cx.update_window(settings.into(), |_, window, _| {
        assert!(window.is_window_active());
    })
    .expect("settings keep window focus");
    close_settings(cx);
}

#[gpui_kit::test]
fn native_close_flushes_valid_text_before_the_debounce_deadline(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let path = fixture._appearance.path().join("settings.json");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.open_settings(window, cx));
    })
    .expect("open settings");
    let settings = settings_window(cx);
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-fonts", cx);
        window.render_frame(cx);
        window.click("appearance-ui-size", cx);
        window.input("27", cx);
    })
    .expect("type a value without waiting for autosave");
    let before: basr::config::Settings =
        serde_json::from_slice(&std::fs::read(&path).expect("original")).expect("preferences");
    assert_eq!(before.appearance.ui_font_size, None);
    let mut visual = gpui_kit::VisualTestContext::from_window(settings.into(), cx);
    assert!(
        visual.simulate_close(),
        "native close callback accepts closure"
    );
    let after: basr::config::Settings =
        serde_json::from_slice(&std::fs::read(&path).expect("saved")).expect("preferences");
    assert_eq!(after.appearance.ui_font_size, Some(27.));
    cx.update(|cx| assert_eq!(Theme::global(cx).font_size, px(27.)));
}

#[gpui_kit::test]
fn settings_open_a_single_separate_window_without_replacing_the_workspace(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    let original_size = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.input("未发送的草稿", cx);
            let application_menu = window.find("app-menu").bounds();
            let workspace = window.find("add-workspace-button").bounds();
            assert!(application_menu.right() <= workspace.left());
            assert_eq!(application_menu.top(), workspace.top());
            assert!(window.find("archived-list").bounds().top() >= workspace.bottom());
            assert!(application_menu.size.width > px(0.));
            assert!(window.try_find("open-settings").is_none());
            assert!(window.try_find("project-logo-slot").is_none());
            assert!(window.try_find("sidebar-connection").is_none());
            assert!(window.try_find("connection-details").is_none());
            let original_size = window.bounds().size;
            window.click("app-menu", cx);
            window.render_frame(cx);
            window.within("popup-menu").click(3usize, cx);
            window.render_frame(cx);
            assert!(window.try_find("settings-page-host").is_none());
            assert!(window.try_find("conversation-connection").is_some());
            assert_eq!(window.bounds().size, original_size);
            original_size
        })
        .expect("workspace window");
    let settings = settings_window(cx);
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-window").bounds().size.width >= px(720.));
        assert_ne!(window.bounds().size.width, original_size.width);
        window.press("ctrl-n", cx);
        assert_eq!(fixture.view.read(cx).active, Some(meta.id));
    })
    .expect("standalone settings");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.open_settings(window, cx));
    })
    .expect("reuse settings window");
    assert_eq!(settings.window_id(), settings_window(cx).window_id());
    cx.update(|cx| assert_eq!(cx.windows().len(), 2));
    close_settings(cx);
    assert!(cx.update_window(settings.into(), |_, _, _| ()).is_err());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.bounds().size, original_size);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "未发送的草稿"
        );
        assert_eq!(fixture.view.read(cx).active, Some(meta.id));
        fixture
            .view
            .update(cx, |view, cx| view.open_settings(window, cx));
    })
    .expect("workspace survives settings closure");
    assert_ne!(settings.window_id(), settings_window(cx).window_id());
}

#[gpui_kit::test]
fn settings_do_not_resize_compact_or_hidden_sidebar_workspaces(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    for compact in [false, true] {
        let original = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.compact = compact;
                    view.sidebar_visible = false;
                    view.resize_window(window, cx);
                    view.focus_input(window, cx);
                });
                window.render_frame(cx);
                assert!(window.try_find("project-logo-slot").is_none());
                let application_menu = window.find("app-menu").bounds();
                let workspace = window.find("workspace-picker").bounds();
                assert!(application_menu.right() <= workspace.left());
                assert_eq!(application_menu.center().y, workspace.center().y);
                assert!(window.try_find("open-settings").is_none());
                let original = window.bounds().size;
                window.press("ctrl-,", cx);
                original
            })
            .expect("open settings from either workspace mode");
        let settings = settings_window(cx);
        cx.update_window(settings.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.bounds().size.height >= px(480.));
        })
        .expect("usable settings size");
        close_settings(cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.bounds().size, original);
            assert_eq!(fixture.view.read(cx).compact, compact);
        })
        .expect("workspace geometry unchanged");
    }
}

#[gpui_kit::test]
fn settings_focus_keeps_background_replies_unread_and_late_updates_do_not_activate_workspace(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("activate workspace");
    cx.executor().run_until_parked();
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.begin_rename(window, cx);
            view.rename.update(cx, |input, cx| {
                input.set_value("title saved before settings", window, cx);
            });
            view.save_title(cx);
            view.open_settings(window, cx);
        });
    })
    .expect("open settings");
    cx.executor().run_until_parked();
    let mut released = false;
    while let Ok(command) = fixture.commands.try_recv() {
        if matches!(command, Command::Foreground(None)) {
            released = true;
        }
    }
    assert!(
        released,
        "the settings window is not a foreground conversation"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Renamed {
                    session: meta.id,
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            view.focus_input(window, cx);
            for event in completed_events(meta.id, "后台完成") {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event,
                    },
                    window,
                    cx,
                );
            }
        });
        window.render_frame(cx);
        assert!(!window.is_window_active());
        assert!(fixture.view.read(cx).has_unread_reply());
    })
    .expect("late workspace updates");
    let settings = settings_window(cx);
    cx.update_window(settings.into(), |_, window, _| {
        assert!(window.is_window_active());
    })
    .expect("settings retain OS focus");
    close_settings(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("return to workspace");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).has_unread_reply());
    })
    .expect("visible reply becomes read");
}

#[gpui_kit::test]
fn settings_window_survives_the_workspace_that_opened_it(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.open_settings(window, cx));
    })
    .expect("open settings");
    let settings = settings_window(cx);
    cx.update_window(fixture.window.into(), |_, window, _| window.remove_window())
        .expect("close only workspace");
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-window").bounds().size.height >= px(480.));
    })
    .expect("settings remain independently usable");
    close_settings(cx);
    cx.update(|cx| assert!(cx.windows().is_empty()));
}

#[gpui_kit::test]
fn successful_agent_login_retries_initialization_and_preserves_the_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.creating = false;
            view.authentication_required = Some("embedded".into());
            view.composer.update(cx, |input, cx| {
                input.set_value("登录后继续的草稿", window, cx)
            });
        });
    })
    .expect("authentication draft");
    cx.update(|cx| {
        cx.update_global::<crate::settings::AgentChanges, _>(|changes, _| {
            changes.server = Some("127.0.0.1:7337".parse().expect("fixture server"));
            changes.ready = Some("embedded".into());
            changes.revision += 1;
        });
    });
    cx.executor().run_until_parked();
    let mut retried = false;
    while let Ok(command) = fixture.commands.try_recv() {
        if let Command::Create(request) = command {
            assert_eq!(request.agent, "embedded");
            retried = true;
        }
    }
    assert!(
        retried,
        "successful login automatically retries the pending session"
    );
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert!(view.authentication_required.is_none());
        assert_eq!(view.composer.read(cx).value(), "登录后继续的草稿");
    });
}

#[gpui_kit::test]
fn an_empty_agent_picker_still_opens_the_installation_catalogue(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let popup = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.agents.clear();
                view.creating = false;
                view.selected_agent.clear();
                view.toggle_mode(window, cx);
            });
            window.bounds_changed(cx);
            window.render_frame(cx);
            window.click("agent-picker", cx);
            fixture
                .view
                .read(cx)
                .menu
                .as_ref()
                .expect("empty picker remains actionable")
                .window
                .expect("agent popup")
        })
        .expect("empty picker");
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .expect("installation entry");
    let settings = settings_window(cx);
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agents-registry").is_some());
    })
    .expect("catalogue opened from chat");
    close_settings(cx);
}

#[gpui_kit::test]
fn installation_catalog_starts_a_new_agent_chat_and_preserves_the_old_draft(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let previous = session();
    attach(&fixture, previous.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("previous thread draft", window, cx)
            });
            cx.update_global::<crate::settings::AgentChanges, _>(|changes, _| {
                changes.target = Some(view.window.window_id());
            });
        });
    })
    .expect("existing chat recorded as settings target");
    while fixture.commands.try_recv().is_ok() {}
    cx.update(|cx| {
        cx.update_global::<crate::settings::AgentChanges, _>(|changes, _| {
            changes.server = Some("127.0.0.1:7337".parse().expect("fixture server"));
            changes.ready = Some("codex-acp".into());
            changes.select = Some("codex-acp".into());
            changes.revision += 1;
        });
    });
    cx.run_until_parked();
    let mut created = None;
    let mut detached = false;
    while let Ok(command) = fixture.commands.try_recv() {
        match command {
            Command::Refresh => {}
            Command::Detach => detached = true,
            Command::Create(request) => {
                assert!(
                    created.is_none(),
                    "explicit start creates exactly one new chat"
                );
                assert_eq!(request.agent, "codex-acp");
                created = Some(request);
            }
            Command::Foreground(_) => {}
            _ => panic!("starting a chat never prompts or modifies the old session"),
        }
    }
    assert!(detached);
    let created = created.expect("explicit catalogue action starts a new session");
    cx.update(|cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.active, None);
        assert_eq!(view.selected_agent, "codex-acp");
        assert!(view.creating);
        assert!(view.composer.read(cx).value().is_empty());
        assert_eq!(view.drafts[&previous.id].text, "previous thread draft");
        assert_eq!(
            view.sessions
                .iter()
                .find(|meta| meta.id == previous.id)
                .expect("old session kept")
                .agent
                .as_str(),
            "embedded"
        );
    });
    let mut next = session();
    next.agent = "codex-acp".into();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: created.epoch,
                    result: Ok(next.clone()),
                },
                window,
                cx,
            );
            assert_eq!(view.active, Some(next.id));
            assert_eq!(
                view.current().expect("new session").agent.as_str(),
                "codex-acp"
            );
            view.select(previous.id, window, cx);
            assert_eq!(view.composer.read(cx).value(), "previous thread draft");
        });
    })
    .expect("old and new chats retain separate agents and drafts");
}
