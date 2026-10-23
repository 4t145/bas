//! Window-bound hints for text supplied by agents, paths, and errors.

use crate::ui;
use gds_uim::el;
use gpui_kit::base::StyledExt;
use gpui_kit::{
    AnyElement, AnyView, App, Div, FontWeight, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, TestSupportExt, Window, component::ActiveTheme,
    component::tooltip::Tooltip, div, prelude::*, px,
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
    el! {
        .div {
            # id;
            hoverable_tooltip: move (window, cx) => (~text, window, cx).view;
        } [child]
    }
}

pub(crate) fn view(text: impl Into<SharedString>, window: &mut Window, cx: &mut App) -> AnyView {
    view_with_id("bounded-tooltip-content", text, window, cx)
}

pub(crate) fn view_with_id(
    id: &'static str,
    text: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let text = text.into();
    let content = wrap_tokens(&text);
    element(
        id,
        text,
        move |_, _| el! { .div { whitespace_normal; } [~content] }.into_any_element(),
        window,
        cx,
    )
}

/// Attach a custom hint whose rows are built whenever the tooltip is shown.
pub(crate) fn element(
    id: &'static str,
    label: impl Into<SharedString>,
    content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let viewport = window.viewport_size();
    let width = (f32::from(viewport.width) - EDGE_SPACE).clamp(1., MAX_WIDTH);
    let height = (f32::from(viewport.height) - EDGE_SPACE).clamp(1., MAX_HEIGHT);
    let label = label.into();
    let style = crate::theme::font_style(cx);
    Tooltip::element(move |window, cx| {
        el! {
            .div {
                # id;
                test_support;
                role: gpui_kit::Role::Label;
                aria_label: ~label;
                max_w: $px((width - 16.).max(1.));
                max_h: $px((height - 12.).max(1.));
                overflow_y_scroll;
                text_size: ui::SMALL;
                whitespace_normal;
            } [(window, cx).content]
        }
    })
    .refine_style(&style)
    .font_family(cx.theme().font_family.clone())
    .font_weight(FontWeight::NORMAL)
    .text_size(ui::SMALL)
    .line_height(gpui_kit::TextStyle::default().line_height)
    .max_w(px(width))
    .max_h(px(height))
    .build(window, cx)
}
