use super::*;
use bas::config::ConnectionProfile;

#[gpui_kit::test]
fn footer_switches_backends_in_place_and_restores_drafts(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let original_session = session();
    attach(&fixture, original_session.clone(), cx);
    let old = fixture
        .view
        .read_with(cx, |view, _| view.backend.server.clone());
    let other: Target = bas::service::Target::Pipe("other-backend".into()).into();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    cx.update(|cx| {
        crate::agents::init(old.clone(), runtime.handle().clone(), cx);
        crate::connections::init_windows(runtime.handle().clone(), "/unused".into(), false, cx);
        crate::connections::register(old.clone(), fixture.window, cx);
        crate::connections::shared(cx).update(cx, |profiles, cx| {
            for (id, name, options) in [
                ("old", "My workstation", &old),
                ("other", "Other backend", &other),
            ] {
                profiles
                    .put(
                        ConnectionProfile {
                            id: id.into(),
                            name: name.into(),
                            target: options.target.clone(),
                            tls: options.tls.clone(),
                            note: None,
                        },
                        cx,
                    )
                    .expect("profile");
            }
        });
    });
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("activate original backend");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = true;
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("Keep my draft", window, cx));
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("server-status-name").label(),
            Some("My workstation")
        );
        let indicator = window.find("server-status-indicator");
        assert_eq!(
            indicator.label(),
            Some(crate::i18n::tr!("status-connected"))
        );
        assert_eq!(indicator.bounds().size.width, px(6.));
        let footer = window.find("server-status");
        assert!(footer.bounds().size.width <= px(SIDEBAR_WIDTH));
        assert!(footer.label().unwrap().contains("TCP"));
        window.click("server-status", cx);
        window.render_frame(cx);
        let menu_entry = window.find("connection-menu-old");
        assert!(
            menu_entry
                .label()
                .unwrap()
                .contains(crate::i18n::tr!("connections-tcp-security"))
        );
        assert!(menu_entry.bounds().size.width <= px(crate::ui::MENU_WIDTH));
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .expect("footer menu");
    cx.run_until_parked();
    let mut presence = Vec::new();
    while let Ok(command) = fixture.commands.try_recv() {
        if let Command::Foreground(session) = command {
            presence.push(session);
        }
    }
    assert!(presence.contains(&Some(original_session.id)));
    assert_eq!(
        presence.last(),
        Some(&None),
        "hidden backend releases foreground"
    );
    cx.update(|cx| {
        assert_eq!(cx.windows().len(), 1, "selection must stay in this window");
        assert_eq!(crate::connections::current(cx), Some(other.clone()));
        assert_eq!(
            crate::connections::shared(cx)
                .read(cx)
                .selected()
                .unwrap()
                .id,
            "other"
        );
        assert_eq!(fixture.view.read(cx).backend.server, old);
        assert!(!fixture.view.read(cx).window.presented);
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "Keep my draft"
        );
    });
    let second = cx
        .update_window(fixture.window.into(), |root, window, cx| {
            let active = active(root, cx);
            assert_eq!(active.read(cx).backend.server, other);
            assert!(active.read(cx).window.presented);
            active.update(cx, |view, cx| {
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("Other service draft", window, cx)
                });
            });
            let size = window.bounds().size;
            let focus = window.focused(cx);
            let events = completed_events(original_session.id, "A reply while hidden");
            fixture.view.update(cx, |view, cx| {
                for event in &events {
                    view.update(
                        Update::Event {
                            epoch: view.selected.epoch,
                            event: event.clone().into(),
                        },
                        window,
                        cx,
                    );
                }
                view.resize_window(window, cx);
                view.sync_foreground(window);
                view.focus_input(window, cx);
                view.operation_error(
                    "Background operation",
                    "Keep this error with its service".into(),
                    cx,
                );
                assert!(view.has_unread_reply(), "hidden replies remain unread");
                assert!(view.backend.foreground.is_none());
            });
            assert_eq!(
                window.focused(cx),
                focus,
                "hidden completions cannot steal focus"
            );
            assert_eq!(cx.windows().len(), 1, "hidden errors do not open a dialog");
            assert_eq!(
                window.bounds().size,
                size,
                "hidden updates cannot resize the window"
            );
            active
        })
        .expect("same window now presents the other backend");
    cx.update(|cx| {
        crate::connections::choose("old", fixture.window.into(), cx).expect("switch back");
        assert_eq!(cx.windows().len(), 1);
    });
    cx.update_window(fixture.window.into(), |root, window, cx| {
        let restored = active(root, cx);
        assert_eq!(restored.entity_id(), fixture.view.entity_id());
        assert!(restored.read(cx).window.presented);
        assert!(!second.read(cx).window.presented);
        window.render_frame(cx);
        assert_eq!(
            window.find("server-status-name").label(),
            Some("My workstation")
        );
        assert_eq!(
            restored.read(cx).composer.input.read(cx).value(),
            "Keep my draft"
        );
    })
    .expect("restored workspace");
    cx.update(|cx| {
        assert_eq!(
            crate::connections::open_for(other, cx)
                .expect("open cached backend")
                .window_id(),
            fixture.window.window_id()
        );
        assert_eq!(
            second.read(cx).composer.input.read(cx).value(),
            "Other service draft"
        );
        assert!(second.read(cx).window.presented);
        assert!(!fixture.view.read(cx).window.presented);
        assert_eq!(cx.windows().len(), 1);
    });
    let cached = second.downgrade();
    drop(second);
    cx.update(|cx| {
        crate::connections::choose("old", fixture.window.into(), cx).expect("return to first");
    });
    cx.update_window(fixture.window.into(), |_, window, _| window.remove_window())
        .expect("close workspace window");
    cx.run_until_parked();
    cx.update(|_| {
        assert!(
            cached.upgrade().is_none(),
            "closing drops hidden workspaces"
        )
    });
}

#[gpui_kit::test]
fn folder_picker_waits_for_its_workspace_to_be_presented(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .expect("active window");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.choose_local_workspace(window, cx);
            view.set_presented(false, window, cx);
        });
        window.render_frame(cx);
    })
    .expect("hide before deferred picker opens");
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_paths());
    cx.update(|cx| {
        let picker = &fixture.view.read(cx).navigation.picker;
        assert!(picker.pending && picker.waiting_activation);
    });
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.show_in_window(false, window, cx));
        window.render_frame(cx);
    })
    .expect("return to picker owner");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
    })
    .expect("run the scheduled picker handoff");
    cx.run_until_parked();
    assert!(
        cx.did_prompt_for_paths(),
        "pending picker resumes instead of becoming stuck"
    );
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert!(!fixture.view.read(cx).navigation.picker.pending));
}

fn active(root: gpui_kit::AnyView, cx: &App) -> Entity<Workspace> {
    let root = root.downcast::<Root>().expect("window root");
    crate::diagnostics::content(root.read(cx), cx)
        .downcast::<crate::connections::WorkspaceHost>()
        .expect("workspace host")
        .read(cx)
        .active()
}
