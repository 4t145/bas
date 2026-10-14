use super::*;
use gpui_kit::{Context, Render, div, px};
use std::{cell::Cell, rc::Rc};

struct Harness {
    disabled: bool,
    loading: bool,
    clicks: Rc<Cell<usize>>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        div().tab_group().child(
            Button::new("action")
                .label("Action")
                .size(px(100.))
                .disabled(self.disabled)
                .loading(self.loading)
                .cursor_pointer()
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
        )
    }
}

#[gpui_kit::test]
fn rendered_button_preserves_activation_and_state_recovery(cx: &mut gpui_kit::TestAppContext) {
    cx.update(gpui_kit::init);
    let clicks = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view({
        let clicks = clicks.clone();
        move |_, _| Harness {
            disabled: false,
            loading: false,
            clicks,
        }
    });
    let mut expected_clicks = 0;
    for (disabled, loading) in [
        (false, false),
        (true, false),
        (false, false),
        (false, true),
        (true, true),
        (false, false),
    ] {
        view.update(cx, |view, cx| {
            view.disabled = disabled;
            view.loading = loading;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.blur(cx);
            window.draw(cx).clear(cx);
        });
        cx.simulate_click(gpui_kit::point(px(10.), px(10.)), Default::default());
        cx.update(|window, cx| window.focus_next(cx));
        for key in ["enter", "space"] {
            let keystroke = gpui_kit::Keystroke::parse(key).expect("activation key");
            cx.simulate_event(gpui_kit::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(gpui_kit::KeyUpEvent { keystroke });
        }
        if !disabled && !loading {
            expected_clicks += 3;
        }
        assert_eq!(clicks.get(), expected_clicks);
    }
}

#[test]
fn cursor_tracks_final_disabled_and_loading_state() {
    for disabled in [false, true] {
        for loading in [false, true] {
            let expected = if disabled || loading {
                CursorStyle::Arrow
            } else {
                CursorStyle::PointingHand
            };
            for mut component in [
                Button::new("button")
                    .disabled(disabled)
                    .loading(loading)
                    .into_component(),
                Button::new("button")
                    .disabled(disabled)
                    .loading(loading)
                    .cursor_pointer()
                    .into_component(),
                Button::new("button")
                    .cursor_pointer()
                    .loading(!loading)
                    .disabled(!disabled)
                    .loading(loading)
                    .disabled(disabled)
                    .into_component(),
                Button::new("button")
                    .link()
                    .disabled(disabled)
                    .loading(loading)
                    .into_component(),
                Button::new("button")
                    .text()
                    .disabled(disabled)
                    .loading(loading)
                    .into_component(),
            ] {
                assert_eq!(component.style().mouse_cursor, Some(expected));
            }
        }
    }
}

#[test]
fn enabled_defaults_to_pointer_but_passive_style_is_preserved() {
    assert_eq!(
        Button::new("action").into_component().style().mouse_cursor,
        Some(CursorStyle::PointingHand)
    );
    assert_eq!(
        Button::new("action")
            .disabled(true)
            .loading(true)
            .loading(false)
            .disabled(false)
            .into_component()
            .style()
            .mouse_cursor,
        Some(CursorStyle::PointingHand)
    );
    assert_eq!(
        Button::new("status")
            .cursor_default()
            .loading(true)
            .disabled(true)
            .disabled(false)
            .loading(false)
            .into_component()
            .style()
            .mouse_cursor,
        Some(CursorStyle::Arrow)
    );
}
