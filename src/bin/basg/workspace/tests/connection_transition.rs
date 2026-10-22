use super::*;

#[gpui_kit::test]
fn initial_connection_is_visible_even_with_an_expanded_composer(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.backend.error = None;
            view.backend.agents.clear();
            view.selected.creating = false;
            view.composer.expanded = true;
            assert!(view.awaiting_connection());
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("server-status-indicator").label(),
            Some(crate::i18n::tr!("status-connecting"))
        );
        assert!(window.find("server-status-loading").visible());
        for width in [1800., 650.] {
            window.resize(size(px(width), px(800.)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            let transition = window.find("server-connection-transition");
            assert!(transition.visible());
            assert_eq!(
                transition.label(),
                Some(crate::i18n::tr!("status-connecting"))
            );
            assert!(window.find("server-connection-loading").visible());
            assert!(window.find("server-connection-target").visible());
            assert!(window.try_find("empty-conversation-message").is_none());
            let bounds = transition.bounds();
            let composer = window.find("composer-panel").bounds();
            assert!(bounds.size.height > px(0.));
            assert!(bounds.bottom() <= composer.top());
            assert_eq!(bounds.left(), composer.left());
            assert_eq!(bounds.right(), composer.right());
        }
    })
    .expect("initial connection layout");
}

#[gpui_kit::test]
fn connection_retry_shows_copyable_failure_and_success_stops_motion(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let failure = "TCP connection refused\n127.0.0.1:4321";
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.backend.error = Some(failure.into());
            view.selected.creating = false;
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("server-status-indicator").label(),
            Some(crate::i18n::tr!("status-reconnecting"))
        );
        assert_eq!(
            window.find("server-connection-transition").label(),
            Some(crate::i18n::tr!("status-reconnecting"))
        );
        assert!(window.find("server-status-loading").visible());
        assert!(window.find("server-connection-loading").visible());
        assert_eq!(
            window.find("server-connection-error").label(),
            Some(failure)
        );
        window.click("server-connection-error-copy", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(failure.into())
        );
        fixture.view.update(cx, |view, cx| {
            view.backend.connection_ready();
            assert!(!view.awaiting_connection());
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("server-connection-transition").is_none());
        assert!(window.try_find("server-status-loading").is_none());
        let indicator = window.find("server-status-indicator");
        assert_eq!(
            indicator.label(),
            Some(crate::i18n::tr!("status-connected"))
        );
        assert_eq!(indicator.bounds().size.width, px(6.));
    })
    .expect("retry and recovery");
}

#[gpui_kit::test]
fn reconnect_keeps_existing_messages_and_draft_visible(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update(|cx| {
        let server = fixture.view.read(cx).backend.server.clone();
        crate::connections::shared(cx).update(cx, |profiles, cx| {
            profiles
                .put(
                    bas::config::ConnectionProfile {
                        id: "reconnecting".into(),
                        name: "Slow server".into(),
                        target: server.target,
                        tls: server.tls,
                        note: None,
                    },
                    cx,
                )
                .expect("connection profile");
        });
    });
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let message_id = fixture.view.update(cx, |view, cx| {
            view.composer.expanded = false;
            for event in completed_events(meta.id, "Keep this answer readable") {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: event.into(),
                    },
                    window,
                    cx,
                );
            }
            view.composer.input.update(cx, |input, cx| {
                input.set_value("Keep my unsent draft", window, cx);
            });
            view.selected
                .timeline
                .conversation
                .as_ref()
                .expect("conversation")
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Message { message, .. } if message.role == bas::model::Role::Agent => {
                        Some(SharedString::from(format!("message-surface-{}", item.id())))
                    }
                    _ => None,
                })
                .expect("agent message")
        });
        window.render_frame(cx);
        assert!(window.find(message_id.clone()).visible());
        fixture.view.update(cx, |view, cx| {
            view.backend.connected = false;
            view.backend.error = Some("Network unreachable".into());
            assert!(
                !view.awaiting_connection(),
                "selected content must remain visible"
            );
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find(message_id).visible());
        assert!(window.try_find("server-connection-transition").is_none());
        assert!(window.find("server-status-loading").visible());
        assert_eq!(
            window.find("server-status-indicator").label(),
            Some(crate::i18n::tr!("status-reconnecting"))
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .composer
                .input
                .read(cx)
                .value()
                .as_ref(),
            "Keep my unsent draft"
        );
        window.click("server-status", cx);
        window.render_frame(cx);
        assert!(
            window.find("connection-menu-reconnecting").visible(),
            "connection menu remains usable"
        );
    })
    .expect("reconnect with retained content");
}
