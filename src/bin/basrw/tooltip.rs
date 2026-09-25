//! Window-bound hints for text supplied by agents, paths, and errors.

use gpui_kit::{
    AnyView, App, Div, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, TestSupportExt, Window, component::tooltip::Tooltip, div,
    prelude::*, px,
};
use unicode_segmentation::UnicodeSegmentation;

const EDGE_SPACE: f32 = 32.;
const MAX_WIDTH: f32 = 360.;
const MAX_HEIGHT: f32 = 320.;
const WORD_SEGMENT: usize = 16;

// GPUI wraps at whitespace, but not necessarily inside a path or an error token.
// Insert soft wrap opportunities without changing the visible characters.
pub(crate) fn wrap_tokens(text: &str) -> String {
    let mut result = String::new();
    let mut run = 0;
    for grapheme in text.graphemes(true) {
        if grapheme.chars().all(char::is_whitespace) {
            run = 0;
        } else {
            if run == WORD_SEGMENT {
                result.push('\u{200b}');
                run = 0;
            }
            run += 1;
        }
        result.push_str(grapheme);
    }
    result
}

/// Attach a hoverable, scrollable hint to any element, including disabled buttons.
pub(crate) fn bounded(
    id: impl Into<gpui_kit::ElementId>,
    child: impl IntoElement,
    text: impl Into<SharedString>,
) -> Stateful<Div> {
    let text = text.into();
    div()
        .id(id)
        .hoverable_tooltip(move |window, cx| view(text.clone(), window, cx))
        .child(child)
}

pub(crate) fn view(text: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> AnyView {
    let viewport = window.viewport_size();
    let width = (f32::from(viewport.width) - EDGE_SPACE).clamp(1., MAX_WIDTH);
    let height = (f32::from(viewport.height) - EDGE_SPACE).clamp(1., MAX_HEIGHT);
    let content = wrap_tokens(&text.into());
    Tooltip::element(move |_, _| {
        div()
            .id("bounded-tooltip-content")
            .test_support()
            .max_w(px((width - 16.).max(1.)))
            .max_h(px((height - 12.).max(1.)))
            .overflow_y_scroll()
            .whitespace_normal()
            .child(content.clone())
    })
    .max_w(px(width))
    .max_h(px(height))
    .build(window, cx)
}

#[cfg(all(test, feature = "gui-tests"))]
#[path = "tooltip/tests.rs"]
mod tests;
