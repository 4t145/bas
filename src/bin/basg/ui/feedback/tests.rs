use super::*;
use crate::theme;
use gpui_kit::{AppContext, Context, Render, TestAppContext, px, size, test::TestWindowExt};

struct Probe {
    text: String,
    compact: bool,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let feedback = message(
            "feedback",
            self.text.clone(),
            gpui_kit::rgb(0xffffff).into(),
        );
        div().size_full().child(if self.compact {
            feedback.compact()
        } else {
            feedback
        })
    }
}

fn check_copy(cx: &mut TestAppContext, compact: bool) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
    });
    // Long tokens exercise wrapping/truncation; neither may alter copied bytes.
    let text = format!(
        "first line  \r\n\t{}界\nlast line  \n  ",
        "long-token".repeat(80)
    );
    let handle = cx.open_window(size(px(240.), px(180.)), |window, cx| {
        let view = cx.new(|_| Probe {
            text: text.clone(),
            compact,
        });
        theme::window_root(view, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let feedback = window.find("feedback");
        let message = window.find("feedback-message");
        let copy = window.find("feedback-copy");
        assert_eq!(feedback.label(), Some(text.as_str()));
        assert_eq!(message.label(), Some(text.as_str()));
        assert!(copy.visible());
        assert!(copy.bounds().right() <= window.viewport_size().width);
        assert!(message.bounds().right() <= copy.bounds().left());
        if compact {
            assert!(feedback.bounds().size.height < px(60.));
            assert!(message.bounds().size.width > px(0.));
        }
        window.click("feedback-copy", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(text.clone())
        );
    })
    .expect("copy feedback through its rendered button");
}

#[gpui_kit::test]
fn multiline_feedback_copy_preserves_whitespace(cx: &mut TestAppContext) {
    check_copy(cx, false);
}

#[gpui_kit::test]
fn compact_feedback_fits_narrow_layout_and_copies_original(cx: &mut TestAppContext) {
    check_copy(cx, true);
}
