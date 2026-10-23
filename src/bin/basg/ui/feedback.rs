use super::{Button, icon_action};
use crate::i18n::tr;
use gds_uim::el;
use gpui_kit::{
    App, ClipboardItem, ElementId, Hsla, IntoElement, RenderOnce, SharedString, TestSupportExt,
    Window, assets::IconName, div, prelude::*,
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(crate) fn copy_button(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Button {
    let text = text.into();
    icon_action(id, IconName::Copy, tr!("feedback-copy")).on_click(move |_, _, cx| {
        cx.stop_propagation();
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
    })
}

pub(crate) fn message(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    color: Hsla,
) -> Feedback {
    Feedback {
        id: id.into(),
        text: text.into(),
        color,
        compact: false,
        copyable: true,
    }
}

#[derive(IntoElement)]
pub(crate) struct Feedback {
    id: SharedString,
    text: SharedString,
    color: Hsla,
    compact: bool,
    copyable: bool,
}

impl Feedback {
    pub(crate) fn without_copy(mut self) -> Self {
        self.copyable = false;
        self
    }

    pub(crate) fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
}

impl RenderOnce for Feedback {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let hint = self.text.clone();
        let display = if self.compact {
            self.text.replace('\n', " ")
        } else {
            super::tooltip::wrap_tokens(&self.text)
        };
        el! { .div {
            # ~$(self.id); test_support;
            role: gpui_kit::Role::Status; aria_label: ~$(self.text);
            flex; items_start; gap_2; min_w_0; w_full; text_size: super::SMALL;
            min_h: super::CONTROL.gpui_kit::px;
            text_color: $self.color;
            @if (self.compact) items_center; @end;
        } [
            .div {
                # format!("{}-message", self.id); test_support;
                role: gpui_kit::Role::Label; aria_label: ~$(self.text);
                flex_1; min_w_0;
                @if (self.compact) truncate; @end;
                hoverable_tooltip: move (window, cx) => (~hint, window, cx).super::tooltip::view;
            } [display],
            @if (self.copyable)
                (format!("{}-copy", self.id), $self.text).copy_button { flex_shrink_0; },
            @end,
        ] }
    }
}
