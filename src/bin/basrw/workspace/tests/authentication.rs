use super::*;

fn auth_required(fixture: &Fixture) -> Update {
    Update::AuthenticationRequired {
        epoch: fixture.initial_create.epoch,
        agent: fixture.initial_create.agent.clone(),
    }
}

#[gpui_kit::test]
fn authentication_preserves_draft_and_attachments_until_explicit_retry(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let file = fixture._appearance.path().join("context.txt");
    std::fs::write(&file, "fixture").expect("attachment");
    let attachment = Attachment::load(&file).expect("load attachment");
    let id = attachment.id;
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.attachments.push(attachment);
            view.composer
                .update(cx, |input, cx| input.set_value("保留草稿", window, cx));
            view.update(auth_required(&fixture), window, cx);
            assert!(!view.creating);
            assert!(view.error.is_none(), "auth is not a generic failure");
            assert!(view.authentication_required.is_some());
            for _ in 0..3 {
                view.update(
                    Update::Catalog(Ok(connection::Catalog {
                        agents: view.agents.clone(),
                        sessions: vec![],
                    })),
                    window,
                    cx,
                );
                view.prepare_session(cx);
            }
            assert_eq!(view.composer.read(cx).value(), "保留草稿");
            assert_eq!(view.attachments[0].id, id);
        });
    })
    .expect("auth state");
    assert!(
        !std::iter::from_fn(|| fixture.commands.try_recv().ok())
            .any(|command| matches!(command, Command::Create(_)))
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.refresh(cx);
            assert!(view.creating);
            assert!(view.authentication_required.is_none());
            let epoch = view.epoch;
            view.update(auth_required(&fixture), window, cx);
            assert!(
                view.authentication_required.is_none(),
                "late auth failure is stale"
            );
            view.refresh(cx);
            assert_eq!(
                view.epoch, epoch,
                "refresh must not duplicate an in-flight create"
            );
        });
    })
    .expect("explicit retry");
    let creates: Vec<_> = std::iter::from_fn(|| fixture.commands.try_recv().ok())
        .filter_map(|command| {
            if let Command::Create(request) = command {
                Some(request)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(creates.len(), 1);
    assert!(creates[0].epoch > fixture.initial_create.epoch);
}

#[gpui_kit::test]
fn compact_auth_button_stays_inside_window_and_opens_local_agent_settings(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.simulate_window_resize(fixture.window.into(), size(px(420.), px(80.)));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.compact = true;
            view.update(auth_required(&fixture), window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.viewport_size(), size(px(420.), px(80.)));
        let button = window.find("agent-authenticate");
        assert!(button.visible());
        assert!(
            button.bounds().right() <= px(420.),
            "button={:?}, viewport={:?}",
            button.bounds(),
            window.viewport_size()
        );
        assert!(button.bounds().bottom() <= px(80.));
        assert!(button.bounds().left() >= px(0.));
        assert!(fixture.view.read(cx).send_hint(cx).contains("需要认证"));
        window.click("agent-authenticate", cx);
    })
    .expect("local auth entry");
    let settings = cx.update(|cx| crate::settings_window::current(cx).expect("settings opened"));
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("agents-page").visible());
        window.dispatch_action(Box::new(crate::settings_window::CloseWindow), cx);
    })
    .expect("local settings section");
}

#[gpui_kit::test]
fn remote_auth_shows_remote_instructions_and_agent_switch_clears_waiting(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.server = "192.0.2.1:7337".parse().expect("test address");
            view.update(auth_required(&fixture), window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        window.click("agent-authenticate", cx);
        assert!(
            crate::settings_window::current(cx).is_none(),
            "never open local auth for remote daemon"
        );
    })
    .expect("remote instructions");
    let message = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| w.window_id() != fixture.window.window_id())
            .expect("message")
    });
    cx.update_window(message, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("message-window").visible());
        window.click("message-close", cx);
    })
    .expect("remote notice");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.select_agent("other".into(), window, cx);
            assert!(view.authentication_required.is_none());
            view.update(auth_required(&fixture), window, cx);
            assert!(
                view.authentication_required.is_none(),
                "old agent response cannot block new agent"
            );
        });
    })
    .expect("agent switch");
}
