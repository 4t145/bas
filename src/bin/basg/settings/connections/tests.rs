use super::*;
use gpui_kit::{TestAppContext, WindowHandle, component::Root, px, size, test::TestWindowExt};

#[test]
fn connection_icons_are_bundled_in_the_application() {
    use gpui_kit::AssetSource;
    for path in [
        "icons/monitor.svg",
        "icons/server.svg",
        crate::assets::SECURE_CONNECTION,
        "icons/cable.svg",
        "icons/pencil.svg",
    ] {
        assert!(
            crate::assets::Assets
                .load(path)
                .expect("asset lookup")
                .is_some(),
            "{path}"
        );
    }
}

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, WindowHandle<Root>, Entity<Page>) {
    let directory = tempfile::tempdir().expect("settings directory");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, "{}").expect("settings");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
        bind_keys(cx);
    });
    let mut page = None;
    // Minimum settings width (720) minus its sidebar (210).
    let window = cx.open_window(size(px(510.), px(700.)), |window, cx| {
        let view = cx.new(|cx| Page::new(window, cx));
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    (directory, window, page.expect("connections page"))
}

fn assert_list_only(window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    assert!(window.try_find("connection-editor").is_none());
    assert!(window.try_find("connection-save").is_none());
    assert!(window.try_find("connections-message").is_none());
}

#[gpui_kit::test]
fn add_menu_keeps_manual_add_explicit(cx: &mut TestAppContext) {
    let (_directory, window, page) = setup(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click("connection-new", cx);
        assert!(
            page.read(cx).editor.is_none(),
            "plus opens a menu, not an editor"
        );
        window.press("down", cx);
        window.press("enter", cx);
        assert!(matches!(page.read(cx).editor, Some(Editor::Add)));
    })
    .expect("add menu");
}

#[gpui_kit::test]
fn discovery_is_read_only_until_add_and_never_retargets(cx: &mut TestAppContext) {
    use bas::service::Target;
    let (directory, window, page) = setup(cx);
    let path = directory.path().join("settings.json");
    let store = bas::config::Store::at(path.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let remote: bas::service::tls::ConnectionOptions = "tcp://192.0.2.1".parse().expect("remote");
    cx.update(|cx| {
        cx.set_global(crate::agents::catalog::Service::new(
            remote.clone(),
            runtime.handle().clone(),
        ))
    });
    let endpoint = Target::Local.resolved().expect("default endpoint");
    cx.update_window(window.into(), |_, window, cx| {
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            profiles
                .put(
                    ConnectionProfile {
                        id: "remote".into(),
                        name: "Remote".into(),
                        note: Some("keep".into()),
                        target: remote.target.clone(),
                        tls: None,
                    },
                    cx,
                )
                .expect("save remote");
            profiles.select("remote", cx).expect("select remote");
            profiles.remove("local", cx).expect("delete saved local");
        });
        let before = std::fs::read(&path).expect("settings");
        page.update(cx, |page, cx| {
            page.begin_scan(std::future::pending(), window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("connection-discovery-loading").visible());
        assert!(window.try_find("connection-discovered").is_none());
        page.update(cx, |page, cx| {
            page.finish_scan(page.scan_generation, Ok(endpoint.clone()), cx)
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("connection-discovered-endpoint").label(),
            Some(connections::presentation::endpoint(&endpoint).as_str())
        );
        assert_eq!(
            window.find("connection-discovered-name").label(),
            Some("Local")
        );
        assert!(
            window
                .find("connection-discovered-endpoint")
                .bounds()
                .right()
                < window.find("connection-discovered-add").bounds().left()
        );
        assert_eq!(std::fs::read(&path).expect("unchanged settings"), before);
        assert_eq!(connections::current(cx), Some(remote.clone()));
        window.click("connection-discovered-add", cx);
        let saved = store.load().expect("saved").settings.frontend;
        assert_eq!(saved.selected, "remote");
        assert_eq!(saved.connections.len(), 2);
        assert_eq!(saved.connections[0].note.as_deref(), Some("keep"));
        assert_eq!(saved.connections[1].target, Target::Local);
        assert_eq!(saved.connections[1].name, "Local");
        assert_eq!(connections::current(cx), Some(remote.clone()));

        // Both the automatic profile and its explicit resolved address count as added.
        page.update(cx, |page, cx| {
            page.begin_scan(std::future::pending(), window, cx);
            page.finish_scan(page.scan_generation, Ok(endpoint.clone()), cx);
            assert!(page.local_profile(&endpoint, cx).is_some());
        });
        window.click("connection-discovered-add", cx);
        assert_eq!(
            store
                .load()
                .expect("saved")
                .settings
                .frontend
                .connections
                .len(),
            2
        );
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            let mut local = saved.connections[1].clone();
            local.target = endpoint.clone();
            local.name = "My workstation".into();
            profiles.put(local, cx).expect("explicit endpoint");
        });
        assert!(page.read(cx).local_profile(&endpoint, cx).is_some());
        window.render_frame(cx);
        assert_eq!(
            window.find("connection-discovered-name").label(),
            Some("My workstation")
        );
        window.click("connection-discovered-add", cx);
        assert_eq!(
            store
                .load()
                .expect("saved")
                .settings
                .frontend
                .connections
                .len(),
            2
        );
    })
    .expect("discovery window");
}

#[gpui_kit::test]
fn discovery_cancel_failure_and_stale_results_cannot_modify_editor(cx: &mut TestAppContext) {
    let (directory, window, page) = setup(cx);
    let path = directory.path().join("settings.json");
    let (send, receive) = futures::channel::oneshot::channel::<()>();
    let old = cx
        .update_window(window.into(), |_, window, cx| {
            let old = page.update(cx, |page, cx| {
                page.begin_scan(
                    async move {
                        let _ = receive.await;
                        Err(())
                    },
                    window,
                    cx,
                );
                page.scan_generation
            });
            window.click("connection-discovery-cancel", cx);
            old
        })
        .expect("cancel discovery");
    cx.run_until_parked();
    assert!(send.send(()).is_err(), "dismissal drops the owned request");
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.add(window, cx);
            page.finish_scan(old, Ok(bas::service::Target::Local), cx);
            assert!(page.discovery.is_none());
            assert!(matches!(page.editor, Some(Editor::Add)));
            page.cancel(window, cx);
            page.begin_scan(std::future::pending(), window, cx);
            page.finish_scan(old, Err(()), cx);
            assert!(matches!(page.discovery, Some(discovery::State::Loading)));
            page.finish_scan(page.scan_generation, Err(()), cx);
        });
        window.render_frame(cx);
        assert!(window.find("connection-discovery-retry").visible());
        assert!(window.try_find("connection-discovered").is_none());
        window.press("escape", cx);
        assert!(page.read(cx).discovery.is_none());
        assert_eq!(
            std::fs::read_to_string(&path).expect("untouched settings"),
            "{}"
        );
    })
    .expect("discovery window");
}

#[gpui_kit::test]
fn mtls_editor_requires_file_references_and_preserves_them(cx: &mut TestAppContext) {
    let (directory, window, page) = setup(cx);
    let store = bas::config::Store::at(directory.path().join("settings.json"));
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.add(window, cx);
            page.select_transport(Transport::Mtls, window, cx);
            page.name
                .update(cx, |input, cx| input.set_value("Secure", window, cx));
            page.target.update(cx, |input, cx| {
                input.set_value("workstation:7338", window, cx)
            });
            page.save(window, cx);
            assert!(matches!(page.feedback, Feedback::Error(_)));
            for (input, value) in [
                (&page.tls_ca, "/certs/ca.pem"),
                (&page.tls_certificate, "/certs/client.pem"),
                (&page.tls_key, "/certs/client.key"),
                (&page.tls_server_name, "expected.example"),
            ] {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        });
        window.render_frame(cx);
        assert!(window.find("connection-tls-key").visible());
        page.update(cx, |page, cx| page.save(window, cx));
        let saved = store.load().expect("saved references").settings.frontend;
        assert_eq!(
            saved.selected, "local",
            "saving does not retarget management"
        );
        let profile = saved.connections.last().expect("secure profile");
        assert_eq!(profile.target.to_string(), "mtls://workstation:7338");
        let tls = profile.tls.as_ref().expect("TLS references");
        assert_eq!(
            tls.private_key,
            std::path::PathBuf::from("/certs/client.key")
        );
        page.update(cx, |page, cx| page.edit(profile, window, cx));
        assert_eq!(
            page.read(cx).tls_key.read(cx).value().as_str(),
            "/certs/client.key"
        );
        page.update(cx, |page, cx| {
            page.select_transport(Transport::Tcp, window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("connection-tls-key").is_none());
    })
    .expect("editor window");
}

#[gpui_kit::test]
fn remove_confirmation_rechecks_the_original_profile(cx: &mut TestAppContext) {
    let (directory, window, page) = setup(cx);
    let store = bas::config::Store::at(directory.path().join("settings.json"));
    let original = ConnectionProfile {
        id: "other".into(),
        name: "Original".into(),
        note: None,
        target: bas::service::Target::Pipe("bas-test".into()),
        tls: None,
    };
    cx.update_window(window.into(), |_, window, cx| {
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            profiles.put(original.clone(), cx).unwrap();
        });
        window.click("connection-remove-other", cx);
        assert_eq!(store.load().unwrap().settings.frontend.connections.len(), 2);
    })
    .unwrap();
    let prompt = confirmation(cx, window);
    let mut replacement = original;
    replacement.name = "Changed while confirmation was open".into();
    cx.update(|cx| {
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            profiles.put(replacement.clone(), cx).unwrap();
        });
    });
    cx.update_window(prompt, |_, window, cx| {
        window.click("message-confirm", cx);
    })
    .unwrap();
    let saved = store.load().unwrap().settings.frontend;
    assert_eq!(saved.connections.len(), 2);
    assert!(saved.connections.contains(&replacement));
    assert_eq!(saved.selected, "local");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("connections-message").visible());
    })
    .unwrap();
}

fn confirmation(cx: &mut TestAppContext, owner: WindowHandle<Root>) -> gpui_kit::AnyWindowHandle {
    cx.update(|cx| {
        let others: Vec<_> = cx
            .windows()
            .into_iter()
            .filter(|window| window.window_id() != owner.window_id())
            .collect();
        assert_eq!(others.len(), 1, "one shared application confirmation");
        others[0]
    })
}

#[gpui_kit::test]
fn save_validates_without_selecting_then_explicit_choose_and_confirm_remove(
    cx: &mut TestAppContext,
) {
    let (directory, window, page) = setup(cx);
    let path = directory.path().join("settings.json");
    let store = bas::config::Store::at(path.clone());
    cx.update_window(window.into(), |_, window, cx| {
        assert_list_only(window, cx);
        // This component's test snapshot does not expose its disabled flag.
        // Verify the action is inert instead of interpreting missing metadata.
        window.click("connection-remove-local", cx);
        assert_eq!(cx.windows().len(), 1);
        assert_eq!(store.load().unwrap().settings.frontend.connections.len(), 1);
        page.update(cx, |page, cx| page.focus_page(window, cx));
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-n"
            } else {
                "ctrl-n"
            },
            cx,
        );
        assert!(matches!(page.read(cx).editor, Some(Editor::Add)));
        assert!(window.try_find("connection-target").is_none());
        window.click("connection-save", cx);
        assert!(window.find("connections-message").visible());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        page.update(cx, |page, cx| {
            page.select_transport(Transport::Tcp, window, cx);
            page.name
                .update(cx, |input, cx| input.set_value("Workstation", window, cx));
            page.note
                .update(cx, |input, cx| input.set_value("用于日常开发", window, cx));
            page.target
                .update(cx, |input, cx| input.set_value("not a target", window, cx));
        });
        window.click("connection-save", cx);
        assert!(window.find("connections-message").visible());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        page.update(cx, |page, cx| {
            page.target.update(cx, |input, cx| {
                input.set_value("tcp://192.0.2.10", window, cx)
            });
        });
        window.click("connection-save", cx);
        assert_list_only(window, cx);
    })
    .unwrap();
    let saved = store.load().unwrap().settings.frontend;
    assert_eq!(saved.connections.len(), 2);
    assert_eq!(saved.selected, "local");
    let profile = saved
        .connections
        .iter()
        .find(|profile| profile.id != "local")
        .unwrap()
        .clone();
    assert_eq!(profile.name, "Workstation");
    assert_eq!(profile.note.as_deref(), Some("用于日常开发"));
    assert_eq!(profile.target, "192.0.2.10".parse().unwrap());
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            window
                .find(format!("connection-note-{}", profile.id))
                .label(),
            Some("用于日常开发")
        );
        window.click(format!("connection-edit-{}", profile.id), cx);
        let card = window
            .find(format!("connection-card-{}", profile.id))
            .bounds();
        let editor = window.find("connection-editor").bounds();
        assert!(editor.top() >= card.top());
        assert!(editor.bottom() <= card.bottom());
        page.update(cx, |page, cx| {
            assert_eq!(page.transport, Transport::Tcp);
            assert_eq!(page.target.read(cx).value().as_ref(), "192.0.2.10");
            assert_eq!(page.note.read(cx).value().as_ref(), "用于日常开发");
            page.name
                .update(cx, |input, cx| input.set_value("Renamed", window, cx));
            page.note
                .update(cx, |input, cx| input.set_value("   ", window, cx));
        });
        window.click("connection-save", cx);
        assert_list_only(window, cx);
        let saved = store.load().unwrap().settings.frontend;
        assert_eq!(
            saved.selected, "local",
            "editing an inactive profile must not select it"
        );
        let edited = saved
            .connections
            .iter()
            .find(|item| item.id == profile.id)
            .unwrap();
        assert_eq!(edited.name, "Renamed");
        assert_eq!(edited.note, None);
        assert!(
            window
                .try_find(format!("connection-note-{}", profile.id))
                .is_none()
        );
        window.click(format!("connection-select-{}", profile.id), cx);
        assert_eq!(store.load().unwrap().settings.frontend.selected, profile.id);
        window.click(format!("connection-remove-{}", profile.id), cx);
        assert_eq!(store.load().unwrap().settings.frontend.connections.len(), 2);
    })
    .unwrap();
    let prompt = confirmation(cx, window);
    cx.update_window(prompt, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("confirmation-window").visible());
        window.click("message-confirm", cx);
    })
    .unwrap();
    let remaining = store.load().unwrap().settings.frontend;
    assert_eq!(remaining.connections.len(), 1);
    assert_eq!(remaining.selected, "local");
    cx.update_window(window.into(), |_, window, cx| {
        assert_list_only(window, cx);
        window.click("connection-remove-local", cx);
        assert_eq!(cx.windows().len(), 1);
        assert_eq!(store.load().unwrap().settings.frontend.connections.len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn transport_drafts_survive_switching_and_cancel_never_persists(cx: &mut TestAppContext) {
    let (directory, window, page) = setup(cx);
    let path = directory.path().join("settings.json");
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| page.add(window, cx));
        page.update(cx, |page, cx| {
            page.name
                .update(cx, |input, cx| input.set_value("Draft", window, cx));
            page.note.update(cx, |input, cx| {
                input.set_value("Keep this note", window, cx)
            });
            for (kind, value) in [
                (Transport::Tcp, "192.0.2.10"),
                (Transport::Unix, "/tmp/bas-test.sock"),
                (Transport::Pipe, "bas-test"),
            ] {
                page.select_transport(kind, window, cx);
                assert!(page.target.read(cx).value().is_empty());
                page.target
                    .update(cx, |input, cx| input.set_value(value, window, cx));
            }
            for (kind, value) in [
                (Transport::Local, ""),
                (Transport::Tcp, "192.0.2.10"),
                (Transport::Unix, "/tmp/bas-test.sock"),
                (Transport::Pipe, "bas-test"),
            ] {
                page.select_transport(kind, window, cx);
                assert_eq!(page.target.read(cx).value().as_ref(), value);
                assert_eq!(page.name.read(cx).value().as_ref(), "Draft");
                assert_eq!(page.note.read(cx).value().as_ref(), "Keep this note");
            }
        });
        window.render_frame(cx);
        for id in [
            "connection-new",
            "connection-reload",
            "connection-select-local",
            "connection-edit-local",
            "connection-remove-local",
        ] {
            window.click(id, cx);
            assert!(matches!(page.read(cx).editor, Some(Editor::Add)));
            assert_eq!(cx.windows().len(), 1, "{id} must not open another action");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "{}",
                "{id} must not write"
            );
        }
        window.click("connection-cancel", cx);
        assert_list_only(window, cx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        page.update(cx, |page, cx| page.add(window, cx));
        assert_eq!(page.read(cx).transport, Transport::Local);
        assert!(page.read(cx).name.read(cx).value().is_empty());
        assert!(page.read(cx).note.read(cx).value().is_empty());
        assert!(window.try_find("connection-target").is_none());
        window.press("escape", cx);
        assert_list_only(window, cx);
        window.click("connection-edit-local", cx);
        page.update(cx, |page, cx| {
            page.name.update(cx, |input, cx| {
                input.set_value("Discard rename", window, cx)
            });
        });
        window.press("escape", cx);
        assert_list_only(window, cx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimum_width_keeps_controls_in_bounds_and_icon_actions_named(cx: &mut TestAppContext) {
    let (_directory, window, page) = setup(cx);
    cx.update_window(window.into(), |_, window, cx| {
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            profiles.put(ConnectionProfile {
                id: "long".into(),
                name: "A connection name that should never push its actions off screen".into(),
                note: Some("A long optional note should remain secondary and stay inside its card.".into()),
                target: bas::service::Target::Unix("/run/user/1000/project-with-a-long-name/private-service.sock".into()),
                tls: None,
            }, cx).expect("long profile");
        });
        window.render_frame(cx);
        for id in [
            "connection-new",
            "connection-reload",
            "connection-select-local",
            "connection-edit-local",
            "connection-remove-local",
            "connection-select-long",
            "connection-edit-long",
            "connection-remove-long",
        ] {
            let action = window.find(id);
            assert!(
                action.label().is_some_and(|label| !label.is_empty()),
                "{id}"
            );
            assert!(action.bounds().left() >= px(0.), "{id}");
            assert!(action.bounds().right() <= px(510.), "{id}");
        }
        window.click("connection-edit-local", cx);
        page.update(cx, |page, cx| {
            page.select_transport(Transport::Tcp, window, cx)
        });
        window.render_frame(cx);
        let editor = window.find("connection-editor").bounds();
        for id in [
            "connection-name",
            "connection-kind",
            "connection-target",
            "connection-note",
            "connection-save",
            "connection-cancel",
        ] {
            let control = window.find(id);
            assert!(control.visible(), "{id}");
            assert!(control.bounds().left() >= editor.left(), "{id}");
            assert!(control.bounds().right() <= editor.right(), "{id}");
            assert!(control.bounds().size.width > px(0.), "{id}");
        }
        assert!(
            window
                .find("connection-cancel")
                .label()
                .is_some_and(|label| !label.is_empty())
        );
        assert!(
            window
                .find("connection-type")
                .label()
                .is_some_and(|label| !label.is_empty())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn cards_share_name_endpoint_and_optional_note_rows(cx: &mut TestAppContext) {
    use crate::connections::presentation;
    let (_directory, window, page) = setup(cx);
    cx.update_window(window.into(), |_, window, cx| {
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            for (id, target) in [
                ("tcp", "tcp://hostname"),
                ("secure", "mtls://hostname"),
                ("socket", "unix:///tmp/service.sock"),
                ("pipe", "pipe://bas-test"),
            ] {
                let target = target.parse().expect("target");
                profiles
                    .put(
                        ConnectionProfile {
                            id: id.into(),
                            name: format!("Saved {id}"),
                            note: Some("My note".into()),
                            tls: matches!(target, bas::service::Target::Mtls(_)).then(|| {
                                bas::service::tls::ClientTls {
                                    ca: "/ca.pem".into(),
                                    certificate: "/client.pem".into(),
                                    private_key: "/client.key".into(),
                                    server_name: None,
                                }
                            }),
                            target,
                        },
                        cx,
                    )
                    .expect("save profile");
            }
        });
        window.render_frame(cx);
        for profile in page.read(cx).profiles.read(cx).entries() {
            let title = window.find(format!("connection-title-{}", profile.id));
            let endpoint = window.find(format!("connection-endpoint-{}", profile.id));
            assert_eq!(
                title.label(),
                Some(
                    format!(
                        "{} · {}",
                        profile.name,
                        presentation::tooltip(&profile.target)
                    )
                    .as_str()
                )
            );
            assert_eq!(
                endpoint.label(),
                Some(presentation::endpoint(&profile.target).as_str())
            );
            assert!(endpoint.bounds().top() >= title.bounds().bottom());
            assert!(endpoint.bounds().right() <= px(510.));
            if profile.note.is_some() {
                let note = window.find(format!("connection-note-{}", profile.id));
                assert!(note.bounds().top() >= endpoint.bounds().bottom());
            } else {
                assert!(
                    window
                        .try_find(format!("connection-note-{}", profile.id))
                        .is_none()
                );
            }
        }
        assert!(
            presentation::tooltip(&"tcp://hostname".parse().unwrap())
                .contains(crate::i18n::tr!("connections-tcp-security"))
        );
    })
    .expect("card rows");
}
