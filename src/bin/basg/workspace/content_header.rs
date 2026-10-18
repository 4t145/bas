//! Shared geometry for collapsible activity rows, independent of their contents.

use super::*;

pub(super) fn line_height(cx: &App) -> Pixels {
    px((f32::from(theme::content_font_size(cx)) * 1.4).ceil())
}

pub(super) fn frame(bordered: bool, cx: &App) -> Div {
    // Transparent borders reserve the same space for thoughts and visible tool cards.
    gds_uim::el! {
        #div {
            w_full; min_w_0; rounded_md; border_1; overflow_hidden;
            border_color: $(if bordered { cx.theme().border } else { cx.theme().border.opacity(0.) });
        }
    }
}

impl Workspace {
    pub(super) fn disclosure(
        &self,
        id: &str,
        label: &str,
        expanded: bool,
        index: usize,
        leading: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Button {
        let content = gds_uim::el! {
            #div { flex; items_center; gap_2; flex_1; min_w_0; } [
                ..leading,
                #div {
                    id: $SharedString::from(format!("disclosure-label-{id}")); test_support;
                    flex_1; min_w_0; truncate;
                    text_size: $theme::content_font_size(cx); line_height: $line_height(cx);
                } [$label.to_owned()],
            ]
        }
        .into_any_element();
        self.disclosure_content(id, label, expanded, index, content, cx)
    }

    pub(super) fn disclosure_content(
        &self,
        id: &str,
        label: &str,
        expanded: bool,
        index: usize,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> Button {
        let key = id.to_owned();
        gds_uim::el! {
            Button($SharedString::from(format!("expand-{id}"))) {
                ghost; small; w_full; h_auto; px_2; py_1;
                min_h: $((line_height(cx) + px(8.)).max(px(crate::ui::COMPACT_CONTROL)));
                icon: $(if expanded { IconName::ChevronDown } else { IconName::ChevronRight });
                accessibility_label: $label.to_owned();
                on_prepaint: $self.track_content_header(id);
                on_click: $cx.listener(move |this, _, _, cx| this.toggle_content(&key, index, cx));
            } [$content]
        }
    }

    pub(super) fn content_header(
        &self,
        id: &str,
        disclosure: impl IntoElement,
        status: Option<AnyElement>,
    ) -> AnyElement {
        gds_uim::el! {
            #div {
                id: $format!("content-header-{id}"); test_support; px_2;
                on_prepaint: $self.track_content_header(id);
            } [
                #crate::ui::ContentRow::new($format!("content-row-{id}"), disclosure) [..status],
            ]
        }
        .into_any_element()
    }
}
