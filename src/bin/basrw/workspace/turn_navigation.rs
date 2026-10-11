use basr::model::{ContentBlock, Message, Role};
use gpui_kit::{ListOffset, relative};
use unicode_segmentation::UnicodeSegmentation;

use super::*;

pub(super) const RAIL_WIDTH: f32 = 24.;
const TICK_WIDTH: f32 = 14.;
const END_TICK_WIDTH: f32 = 18.;
const HOVER_TICK_WIDTH: f32 = 20.;
const MARKER_HEIGHT: f32 = 12.;
const POSITION_HEIGHT: f32 = 0.06;
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
        let prompt_count = prompts.len();
        let session = conversation.session.id;
        let mut rail = div()
            .id("turn-navigation")
            .test_support()
            .absolute()
            .top(px(0.))
            .bottom(px(0.))
            .right(px(if self.compact { 0. } else { -RAIL_WIDTH }))
            .flex()
            .flex_col()
            .justify_between()
            .when(prompt_count == 0, |rail| rail.justify_end())
            .w(px(RAIL_WIDTH))
            .flex_shrink_0()
            .bg(cx.theme().muted.opacity(0.24))
            .hover(|rail| rail.bg(cx.theme().muted.opacity(0.36)))
            .border_l_1()
            .border_color(cx.theme().border.opacity(0.8));

        if let Some(thumb) = self.render_turn_position(cx) {
            rail = rail.child(thumb);
        }

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
            rail = rail.child(marker);
        }

        let group = SharedString::from("turn-marker-end");
        rail.child(
            crate::tooltip::bounded(
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
                "跳到对话末尾 · Ctrl+End",
            )
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label("跳到对话末尾")
            .group(group)
            .h(px(MARKER_HEIGHT))
            .w_full()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.scroll_conversation_to_end(cx);
            })),
        )
        .into_any_element()
    }

    fn render_turn_position(&self, cx: &App) -> Option<AnyElement> {
        let row_count = self.list.item_count();
        if row_count == 0 {
            return None;
        }
        let offset = self.list.logical_scroll_top();
        let row_fraction = self
            .list
            .bounds_for_item(offset.item_ix)
            .map(|bounds| f32::from(offset.offset_in_item) / f32::from(bounds.size.height))
            .filter(|fraction| fraction.is_finite())
            .unwrap_or(0.)
            .clamp(0., 1.);
        let progress = if self.list.is_following_tail() {
            1.
        } else {
            ((offset.item_ix as f32 + row_fraction) / row_count as f32).clamp(0., 1.)
        };
        Some(
            div()
                .absolute()
                .right(px(1.))
                .top(relative(progress * (1. - POSITION_HEIGHT)))
                .h(relative(POSITION_HEIGHT))
                .w(px(5.))
                .rounded_full()
                .bg(cx.theme().muted_foreground.opacity(0.55))
                .into_any_element(),
        )
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
        "图片或附件".into()
    } else {
        preview
    }
}
