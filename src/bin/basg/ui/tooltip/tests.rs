use std::time::Duration;

use gpui_kit::{
    Context, IntoElement, Render, ScrollDelta, TestAppContext, Window, div, point, px, size,
    test::TestWindowExt,
};

use super::*;

struct HintFixture {
    text: String,
}

impl Render for HintFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .h_full()
            .flex()
            .items_end()
            .justify_end()
            .child(bounded(
                "hint-trigger",
                div()
                    .id("hint-target")
                    .test_support()
                    .size(px(24.))
                    .child("!"),
                self.text.clone(),
            ))
    }
}

#[gpui_kit::test]
fn delayed_hover_keeps_long_hints_inside_compact_and_expanded_windows(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for dimensions in [
        size(px(420.), px(80.)),
        size(px(420.), px(240.)),
        size(px(1120.), px(800.)),
    ] {
        for text in [
            format!(
                "initialization failure: {}\n\nTry authenticating again.",
                "very-long-unbroken-path/".repeat(60)
            ),
            "first line\nsecond line\n".repeat(70),
        ] {
            let window_handle = cx.open_window(dimensions, |window, cx| {
                let view = cx.new(|_| HintFixture { text });
                gpui_kit::component::Root::new(view, window, cx)
            });
            cx.update_window(window_handle.into(), |_, window, cx| {
                window.render_frame(cx);
                let target = window.find("hint-target").bounds();
                window.simulate_mouse_move(target.center(), cx);
                window.render_frame(cx);
                assert!(
                    window.try_find("bounded-tooltip-content").is_none(),
                    "hint must wait for hover delay"
                );
            })
            .expect("hover fixture");
            cx.executor().advance_clock(Duration::from_millis(600));
            cx.update_window(window_handle.into(), |_, window, cx| {
                window.render_frame(cx);
                let viewport = window.viewport_size();
                let mut tooltip_scope = window.within("gpui_base::tooltip::Tooltip");
                let content = tooltip_scope.find("bounded-tooltip-content").bounds();
                // Upstream popup contributes 8px horizontal padding and a 12px margin.
                assert!(
                    content.left() >= px(8.) && content.top() >= px(0.),
                    "{content:?}"
                );
                assert!(
                    content.right() + px(8.) <= viewport.width
                        && content.bottom() <= viewport.height,
                    "{content:?} outside {viewport:?}"
                );
                tooltip_scope.scroll(
                    "bounded-tooltip-content",
                    ScrollDelta::Pixels(point(px(0.), px(-120.))),
                    cx,
                );
                assert!(tooltip_scope.find("bounded-tooltip-content").visible());
                window.simulate_mouse_move(point(px(0.), px(0.)), cx);
                window.remove_window();
            })
            .expect("measure hint");
        }
    }
}

#[test]
fn long_tokens_have_soft_breaks_without_losing_visible_characters() {
    let text = "👨‍👩‍👧‍👦/path".repeat(30);
    let wrapped = wrap_tokens(&text);
    assert!(wrapped.contains('\u{200b}'));
    assert_eq!(wrapped.replace('\u{200b}', ""), text);
}
