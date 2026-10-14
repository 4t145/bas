use super::*;
use bas::acp;

fn options(checked: bool) -> Vec<acp::SessionConfigOption> {
    vec![
        acp::SessionConfigOption::boolean("fast-mode", "Fast mode", checked)
            .category(acp::SessionConfigOptionCategory::ModelConfig)
            .description("1.5x speed, increased usage"),
        acp::SessionConfigOption::boolean("custom-feature", "Custom option", false),
    ]
}

#[gpui_kit::test]
fn boolean_switch_clicks_send_requested_values_and_wait_for_backend_state(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.acp =
        Some(acp::NewSessionResponse::new(meta.id.to_string()).config_options(options(false)));
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let switch = window.find("config-fast-mode");
        assert_eq!(switch.role(), Some(gpui_kit::Role::Switch));
        assert_eq!(switch.checked(), Some(false));
        assert_eq!(switch.label(), Some("Fast mode"));
        assert_eq!(
            window.find("config-custom-feature").role(),
            Some(gpui_kit::Role::Switch)
        );
        assert!(window.find("config-label-fast-mode").bounds().right() < switch.bounds().left());
        window.click("config-fast-mode", cx);
        window.render_frame(cx);
        assert!(
            fixture
                .view
                .read(cx)
                .composer
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        assert_eq!(
            window.find("config-fast-mode").checked(),
            Some(false),
            "the switch reflects confirmed backend state"
        );
        assert!(
            fixture
                .view
                .read(cx)
                .sessions
                .pending
                .configuring
                .contains(&meta.id)
        );
        window.click("config-fast-mode", cx);
        window.click("config-custom-feature", cx);
    })
    .expect("click controlled switch");
    let Command::SetConfig(request) = fixture.commands.try_recv().expect("option request") else {
        panic!("switch click must only configure its option")
    };
    assert_eq!(request.session_id.to_string(), meta.id.to_string());
    assert_eq!(request.config_id.to_string(), "fast-mode");
    assert_eq!(request.value.as_bool(), Some(true));
    assert!(
        fixture.commands.try_recv().is_err(),
        "pending/disabled toggles must not send prompts or duplicate requests"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(acp::SessionUpdate::ConfigOptionUpdate(
                            acp::ConfigOptionUpdate::new(options(true)),
                        )),
                    ),
                },
                window,
                cx,
            );
            view.update(
                Update::Configured {
                    session: meta.id,
                    result: Ok(acp::SetSessionConfigOptionResponse::new(options(true))),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(window.find("config-fast-mode").checked(), Some(true));
        assert!(
            !fixture
                .view
                .read(cx)
                .sessions
                .pending
                .configuring
                .contains(&meta.id)
        );
        window.click("config-fast-mode", cx);
    })
    .expect("backend confirms changed value");
    let Command::SetConfig(request) = fixture.commands.try_recv().expect("reverse option request")
    else {
        panic!("switch click must only configure its option")
    };
    assert_eq!(request.value.as_bool(), Some(false));
    assert!(
        fixture.commands.try_recv().is_err(),
        "changing options must not send a model prompt"
    );
}

#[gpui_kit::test]
fn fast_mode_name_and_switch_fit_the_compact_minimum_and_keep_toolbar_alignment(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.acp = Some(
        acp::NewSessionResponse::new(meta.id.to_string())
            .config_options(vec![options(false).remove(0)]),
    );
    attach(&fixture, meta, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.window.compact = true;
            cx.notify();
        });
        window.resize(size(px(420.), px(80.)));
        window.render_frame(cx);
        window.render_frame(cx);
        let field = window.find("config-field-fast-mode").bounds();
        let label = window.find("config-label-fast-mode").bounds();
        let switch = window.find("config-fast-mode").bounds();
        let send = window.find("send-message").bounds();
        assert!(label.size.width > px(0.) && switch.size.width >= px(28.));
        assert!(field.left() >= px(0.) && field.right() <= window.viewport_size().width);
        assert!(label.right() < switch.left());
        assert!(f32::from(switch.center().y - send.center().y).abs() <= 0.5);
        assert!(f32::from(label.center().y - switch.center().y).abs() <= 0.5);
    })
    .expect("compact boolean control");
}
