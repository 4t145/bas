use basr::model::{ContentBlock, Message, Role};
use gpui_kit::ListOffset;
use unicode_segmentation::UnicodeSegmentation;

use super::*;

pub(super) const RAIL_WIDTH: f32 = 24.;
const TICK_WIDTH: f32 = 14.;
const END_TICK_WIDTH: f32 = 18.;
const HOVER_TICK_WIDTH: f32 = 20.;
const MARKER_HEIGHT: f32 = 12.;
const VISIBLE_PROMPTS: usize = 20;
const PREVIEW_GRAPHEMES: usize = 72;

impl Workspace {
    pub(super) fn scroll_conversation_to_end(&mut self, cx: &mut Context<Self>) {
        self.list.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    pub(super) fn render_turn_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(conversation) = &self.conversation else {
            return div().into_any_element();
        };
        let prompts: Vec<_> = conversation
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Message { message, .. } if message.role == Role::User => {
                    Some((index, prompt_preview(message)))
                }
                _ => None,
            })
            .collect();
        let session = conversation.session.id;
        let rail = div()
            .id("turn-navigation")
            .test_support()
            .absolute()
            .top(px(0.))
            .bottom(px(0.))
            .right(px(if self.compact {
                0.
            } else {
                -RAIL_WIDTH - chat::THREAD_BODY_PADDING
            }))
            .flex()
            .flex_col()
            .justify_center()
            .py_1()
            .w(px(RAIL_WIDTH))
            .flex_shrink_0();
        let mut markers = div()
            .id(SharedString::from(format!("turn-prompts-{session}")))
            .test_support()
            .w_full()
            .min_h_0()
            .max_h(px(MARKER_HEIGHT * VISIBLE_PROMPTS as f32))
            .overflow_y_scroll();

        for (item_index, prompt) in prompts {
            let marker_id = SharedString::from(format!("turn-marker-{item_index}"));
            let group = marker_id.clone();
            let prompt: SharedString = prompt.into();
            let marker = crate::tooltip::bounded(
                marker_id,
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id(SharedString::from(format!("turn-marker-line-{item_index}")))
                            .test_support()
                            .w(px(TICK_WIDTH))
                            .h(px(2.))
                            .rounded_full()
                            .bg(cx.theme().muted_foreground.opacity(0.7))
                            .group_hover(group.clone(), |tick| {
                                tick.w(px(HOVER_TICK_WIDTH))
                                    .h(px(3.))
                                    .bg(cx.theme().primary)
                            }),
                    ),
                prompt.clone(),
            )
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(prompt)
            .group(group)
            .h(px(MARKER_HEIGHT))
            .flex_shrink_0()
            .w_full()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.active != Some(session) {
                    return;
                }
                if let Some(row) = this
                    .conversation
                    .as_ref()
                    .and_then(|conversation| conversation.row_for_item(item_index))
                {
                    this.list.scroll_to(ListOffset {
                        item_ix: row,
                        offset_in_item: px(0.),
                    });
                    cx.notify();
                }
            }));
            markers = markers.child(marker);
        }

        let group = SharedString::from("turn-marker-end");
        let end = crate::tooltip::bounded(
            group.clone(),
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("turn-marker-line-end")
                        .test_support()
                        .w(px(END_TICK_WIDTH))
                        .h(px(3.))
                        .rounded_full()
                        .bg(cx.theme().primary.opacity(0.85))
                        .group_hover(group.clone(), |tick| {
                            tick.w(px(HOVER_TICK_WIDTH))
                                .h(px(4.))
                                .bg(cx.theme().primary)
                        }),
                ),
            crate::i18n::tr!("conversation-end-shortcut"),
        )
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(crate::i18n::tr!("conversation-end"))
        .group(group)
        .h(px(MARKER_HEIGHT))
        .flex_shrink_0()
        .w_full()
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.scroll_conversation_to_end(cx);
        }));
        rail.child(
            div()
                .id("turn-shortcuts")
                .test_support()
                .flex()
                .flex_col()
                .w_full()
                .max_h_full()
                .rounded_md()
                .hover(|group| group.bg(cx.theme().muted.opacity(0.24)))
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(markers)
                .child(end),
        )
        .into_any_element()
    }
}

fn prompt_preview(message: &Message) -> String {
    let mut preview = String::new();
    let mut graphemes = 0;
    let mut space_pending = false;
    for block in &message.content {
        let ContentBlock::Text(text) = block else {
            continue;
        };
        for grapheme in text.text.graphemes(true) {
            if grapheme.chars().all(char::is_whitespace) {
                space_pending = !preview.is_empty();
                continue;
            }
            if space_pending {
                if graphemes == PREVIEW_GRAPHEMES {
                    preview.push('…');
                    return preview;
                }
                preview.push(' ');
                graphemes += 1;
                space_pending = false;
            }
            if graphemes == PREVIEW_GRAPHEMES {
                preview.push('…');
                return preview;
            }
            preview.push_str(grapheme);
            graphemes += 1;
        }
    }
    if preview.is_empty() {
        crate::i18n::tr!("attachment-message").into()
    } else {
        preview
    }
}
