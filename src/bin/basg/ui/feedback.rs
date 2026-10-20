use gpui_kit::{
    App, ClipboardItem, ElementId, Hsla, IntoElement, RenderOnce, SharedString, TestSupportExt,
    Window, assets::IconName, div, prelude::*,
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(crate) fn copy_button(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
) -> super::Button {
    let text = text.into();
    super::icon_action(id, IconName::Copy, crate::i18n::tr!("feedback-copy")).on_click(
        move |_, _, cx| {
            cx.stop_propagation();
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        },
    )
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
    }
}

#[derive(IntoElement)]
pub(crate) struct Feedback {
    id: SharedString,
    text: SharedString,
    color: Hsla,
    compact: bool,
}

impl Feedback {
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
        gds_uim::el! { #div {
            id: $self.id.clone(); test_support;
            role: gpui_kit::Role::Status; aria_label: $self.text.clone();
            flex; items_start; gap_2; min_w_0; w_full; text_size: super::SMALL;
            min_h: $gpui_kit::px(super::CONTROL);
            text_color: $self.color;
            @$self.compact => { items_center; };
        } [
            #div {
                id: $format!("{}-message", self.id); test_support;
                role: gpui_kit::Role::Label; aria_label: $self.text.clone();
                flex_1; min_w_0;
                @$self.compact => { truncate; };
                hoverable_tooltip: $move |window, cx| super::tooltip::view(hint.clone(), window, cx);
            } [$display],
            #copy_button($format!("{}-copy", self.id), $self.text) { flex_shrink_0; },
        ] }
    }
}
