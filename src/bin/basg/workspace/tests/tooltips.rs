use std::time::Duration;

use gpui_kit::{point, px, size};

use super::*;

#[gpui_kit::test]
fn existing_workspace_button_shows_its_registered_shortcut(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover("add-workspace-button", cx);
    })
    .expect("existing workspace button");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let expected = crate::workspace::shortcuts::hint(
            crate::i18n::tr!("workspace-add"),
            &crate::workspace::shortcuts::AddWorkspace,
            window,
        );
        assert_ne!(expected, crate::i18n::tr!("workspace-add"));
        assert_eq!(
            window.find("bounded-tooltip-content").label(),
            Some(expected.as_str())
        );
    })
    .expect("shortcut hint");
}

#[gpui_kit::test]
fn failed_initialization_send_hint_fits_compact_minimum(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.window.compact = true;
            view.selected.error = Some(format!(
                "Agent initialization failed: {}\n\nSign in again.",
                "/very/long/unbroken/agent/error".repeat(40)
            ));
            cx.notify();
        });
        window.resize(size(px(420.), px(80.)));
        window.render_frame(cx);
        assert!(
            window
                .find("composer-feedback-message")
                .label()
                .expect("visible error")
                .starts_with("Agent initialization failed:")
        );
        let trigger = window.find("send-message").bounds();
        window.simulate_mouse_move(trigger.center(), cx);
        window.render_frame(cx);
        assert!(window.try_find("bounded-tooltip-content").is_none());
    })
    .expect("compact workspace");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let viewport = window.viewport_size();
        let content = window
            .within("gpui_base::tooltip::Tooltip")
            .find("bounded-tooltip-content")
            .bounds();
        let hint = window
            .find("bounded-tooltip-content")
            .label()
            .expect("send hint")
            .to_owned();
        assert!(hint.contains("发送消息"), "{hint}");
        assert!(!hint.contains("initialization"));
        assert!(
            content.left() >= px(8.) && content.top() >= px(0.),
            "{content:?}"
        );
        assert!(
            content.right() + px(8.) <= viewport.width && content.bottom() <= viewport.height,
            "{content:?} outside {viewport:?}"
        );
        window.simulate_mouse_move(point(px(0.), px(0.)), cx);
    })
    .expect("send hint");
}

#[gpui_kit::test]
fn composer_feedback_keeps_full_error_details_accessible_in_both_modes(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let error = format!("Failed to connect\n{}", "/very/long/path".repeat(50));
    for compact in [false, true] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.window.compact = compact;
                view.selected.error = Some(error.clone());
                view.resize_window(window, cx);
                cx.notify();
            });
            window.bounds_changed(cx);
            window.render_frame(cx);
            let message = window.find("composer-feedback-message");
            assert_eq!(message.label(), Some(error.as_str()));
            let details = window.find("composer-feedback-details").bounds();
            assert!(details.left() >= message.bounds().right());
            assert!(details.right() <= window.viewport_size().width);
            window.click("composer-feedback-details", cx);
        })
        .expect("visible feedback and details action");
        let details = cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find(|handle| *handle != fixture.window.into())
                .expect("details window")
        });
        cx.update_window(details, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("application-message-body").visible());
            assert_eq!(
                window.find("application-message-body").label(),
                Some(error.as_str())
            );
            window.click("message-close", cx);
        })
        .expect("full details can be opened and dismissed");
    }
}

#[gpui_kit::test]
fn agent_hint_identifies_the_selected_agent_even_when_selection_is_locked(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    let agent = meta.agent.to_string();
    attach(&fixture, meta, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.hover("agent-picker", cx);
    })
    .expect("hover current agent");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let hint = window
            .find("bounded-tooltip-content")
            .label()
            .expect("agent name")
            .to_owned();
        assert_eq!(
            crate::i18n::visible_text(&hint),
            format!("智能体 · {agent}")
        );
    })
    .expect("current agent information");
}
