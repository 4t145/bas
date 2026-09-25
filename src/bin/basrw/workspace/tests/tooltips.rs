use std::time::Duration;

use gpui_kit::{point, px, size};

use super::*;

#[gpui_kit::test]
fn failed_initialization_send_hint_fits_compact_minimum(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.compact = true;
            view.error = Some(format!(
                "Agent initialization failed: {}\n\nSign in again.",
                "/very/long/unbroken/agent/error".repeat(40)
            ));
            cx.notify();
        });
        window.resize(size(px(420.), px(80.)));
        window.render_frame(cx);
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
