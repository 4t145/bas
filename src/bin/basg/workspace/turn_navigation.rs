use bas::model::{ContentBlock, Message, Role};
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
        self.selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    pub(super) fn render_turn_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(conversation) = &self.selected.timeline.conversation else {
            return gds_uim::el! { #div }.into_any_element();
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
        let rail = gds_uim::el! { #div {
            id: "turn-navigation"; test_support; absolute;
            top: 0.0.px; bottom: 0.0.px;
            right: $px(if self.window.compact {
                0.
            } else {
                -RAIL_WIDTH - chat::THREAD_BODY_PADDING
            });
            flex; flex_col; justify_center; py_1; w: RAIL_WIDTH.px; flex_shrink_0;
        } };
        let mut markers = gds_uim::el! { #div {
            id: $SharedString::from(format!("turn-prompts-{session}"));
            test_support; w_full; min_h_0;
            max_h: $px(MARKER_HEIGHT * VISIBLE_PROMPTS as f32);
            overflow_y_scroll;
        } };

        for (item_index, prompt) in prompts {
            let marker_id = SharedString::from(format!("turn-marker-{item_index}"));
            let group = marker_id.clone();
            let prompt: SharedString = prompt.into();
            let tick = gds_uim::el! {
                #div { size_full; flex; items_center; justify_center; } [
                    #div {
                        id: $SharedString::from(format!("turn-marker-line-{item_index}"));
                        test_support; w: TICK_WIDTH.px; h: 2.0.px; rounded_full;
                        bg: $cx.theme().muted_foreground.opacity(0.7);
                        group_hover: $group.clone(), $gds_uim::m! {
                            w: HOVER_TICK_WIDTH.px; h: 3.0.px; bg: $cx.theme().primary;
                        };
                    }
                ]
            };
            let marker = gds_uim::el! { #crate::ui::tooltip::bounded(
                marker_id,
                tick,
                $prompt.clone(),
            ) {
                test_support; role: gpui_kit::Role::Button;
                aria_label: prompt; group: group; h: MARKER_HEIGHT.px;
                flex_shrink_0; w_full; cursor_pointer;
                on_click: $cx.listener(move |this, _, _, cx| {
                    this.on_turn_navigation_prompt(session, item_index, cx);
                });
            } };
            markers = gds_uim::el! { $markers [$marker] };
        }

        let group = SharedString::from("turn-marker-end");
        let tick = gds_uim::el! {
            #div { size_full; flex; items_center; justify_center; } [
                #div {
                    id: "turn-marker-line-end"; test_support;
                    w: END_TICK_WIDTH.px; h: 3.0.px; rounded_full;
                    bg: $cx.theme().primary.opacity(0.85);
                    group_hover: $group.clone(), $gds_uim::m! {
                        w: HOVER_TICK_WIDTH.px; h: 4.0.px; bg: $cx.theme().primary;
                    };
                }
            ]
        };
        let end = gds_uim::el! { #crate::ui::tooltip::bounded(
            $group.clone(),
            tick,
            $crate::i18n::tr!("conversation-end-shortcut"),
        ) {
            test_support; role: gpui_kit::Role::Button;
            aria_label: $crate::i18n::tr!("conversation-end"); group: group;
            h: MARKER_HEIGHT.px; flex_shrink_0; w_full; cursor_pointer;
            on_click: $cx.listener(Self::on_turn_navigation_end);
        } };
        gds_uim::el! { $rail [
            #div {
                id: "turn-shortcuts"; test_support; flex; flex_col;
                w_full; max_h_full; rounded_md;
                hover => { bg: $cx.theme().muted.opacity(0.24); };
                on_scroll_wheel: $|_, _, cx| cx.stop_propagation();
            } [$markers, $end]
        ] }
        .into_any_element()
    }

    fn on_turn_navigation_end(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll_conversation_to_end(cx);
    }

    fn on_turn_navigation_prompt(
        &mut self,
        session: SessionId,
        item_index: usize,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id != Some(session) {
            return;
        }
        if let Some(row) = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.row_for_item(item_index))
        {
            self.selected.timeline.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
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
