use basr::model::Role;
use gpui_kit::{Bounds, ListOffset};

use super::*;

const ACTION_FOREGROUND_OPACITY: f32 = 0.5;
const ACTION_HOVER_OPACITY: f32 = 0.35;
const ACTION_ACTIVE_OPACITY: f32 = 0.55;

pub(super) fn action_button(
    id: impl Into<gpui_kit::ElementId>,
    icon: IconName,
    cx: &App,
) -> Button {
    let color = cx
        .theme()
        .muted_foreground
        .opacity(ACTION_FOREGROUND_OPACITY);
    layout::icon_button(id, icon)
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(cx.theme().foreground)
                .hover(cx.theme().muted.opacity(ACTION_HOVER_OPACITY))
                .active(cx.theme().muted.opacity(ACTION_ACTIVE_OPACITY)),
        )
        .text_color(color)
}

struct Response {
    text: String,
    user_item: Option<usize>,
    latest: bool,
}

fn response(conversation: &Conversation, index: usize) -> Option<Response> {
    if !is_agent_message(&conversation.items[index]) {
        return (index + 1 == conversation.items.len()).then(|| Response {
            text: conversation
                .items
                .iter()
                .rposition(is_agent_message)
                .map(|index| response_text(conversation, index))
                .unwrap_or_default(),
            user_item: None,
            latest: true,
        });
    }
    let user_item = conversation.items[..index]
        .iter()
        .rposition(is_user_message);
    let end = conversation.items[index + 1..]
        .iter()
        .position(is_user_message)
        .map_or(conversation.items.len(), |offset| index + offset + 1);
    if conversation.items[index + 1..end]
        .iter()
        .any(|item| is_agent_message(item) || matches!(item, Item::Tool(_)))
        || (end == conversation.items.len() && index + 1 != end)
    {
        return None;
    }
    Some(Response {
        text: response_text(conversation, index),
        user_item,
        latest: index + 1 == conversation.items.len(),
    })
}

fn response_text(conversation: &Conversation, index: usize) -> String {
    let start = conversation.items[..index]
        .iter()
        .rposition(is_user_message)
        .map_or(0, |index| index + 1);
    let end = conversation.items[index + 1..]
        .iter()
        .position(is_user_message)
        .map_or(conversation.items.len(), |offset| index + offset + 1);
    conversation.items[start..end]
        .iter()
        .filter_map(|item| match item {
            Item::Message { message, .. } if message.role == Role::Agent => {
                let text = message.plain_text();
                (!text.trim().is_empty()).then_some(text)
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn is_agent_message(item: &Item) -> bool {
    matches!(item, Item::Message { message, .. } if message.role == Role::Agent)
}

fn is_user_message(item: &Item) -> bool {
    matches!(item, Item::Message { message, .. } if message.role == Role::User)
}

impl Workspace {
    pub(super) fn track_content_header(
        &self,
        id: &str,
    ) -> impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static {
        let bounds = self.content_bounds.clone();
        let id = id.to_owned();
        move |header, _, _| {
            bounds.borrow_mut().insert(id, header);
        }
    }

    pub(super) fn render_content_actions(
        &self,
        id: &str,
        collapse_key: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let top_id = id.to_owned();
        let collapse_id = id.to_owned();
        let collapse_key = collapse_key.to_owned();
        let copy = self.conversation.as_ref().and_then(|conversation| {
            conversation.items.iter().find_map(|item| match item {
                Item::Message { message, .. }
                    if item.id() == id && message.role == Role::Thought =>
                {
                    Some(message.plain_text())
                }
                _ => None,
            })
        });
        let group = SharedString::from(format!("content-actions-{id}"));
        div()
            .id(group.clone())
            .test_support()
            .group(group.clone())
            .flex()
            .items_center()
            .justify_end()
            .gap_1()
            .pt_1()
            .when_some(copy, |actions, copy| {
                actions.child(
                    action_button(SharedString::from(format!("copy-{id}")), IconName::Copy, cx)
                        .accessibility_label("复制思考内容")
                        .tooltip("复制思考内容")
                        .invisible()
                        .group_hover(group, |style| style.visible())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                        }),
                )
            })
            .child(
                action_button(
                    SharedString::from(format!("scroll-to-content-{id}")),
                    IconName::CornerLeftUp,
                    cx,
                )
                .accessibility_label("返回内容顶部")
                .tooltip("返回内容顶部")
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let offset = this.content_scroll_offset(&top_id, index);
                    this.list.scroll_to(offset);
                    cx.notify();
                })),
            )
            .child(
                action_button(
                    SharedString::from(format!("collapse-content-{id}")),
                    IconName::ChevronUp,
                    cx,
                )
                .accessibility_label("收起内容")
                .tooltip("收起内容")
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let offset = this.content_scroll_offset(&collapse_id, index);
                    this.expanded.remove(&collapse_key);
                    this.list.remeasure_items(index..index + 1);
                    this.list.scroll_to(offset);
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    pub(super) fn content_scroll_offset(&self, id: &str, index: usize) -> ListOffset {
        let offset_in_item = self
            .content_bounds
            .borrow()
            .get(id)
            .zip(self.list.bounds_for_item(index))
            .map_or(px(0.), |(header, row)| {
                (header.top() - row.top()).max(px(0.))
            });
        ListOffset {
            item_ix: index,
            offset_in_item,
        }
    }

    pub(super) fn render_message_actions(
        &self,
        item: &Item,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let conversation = self.conversation.as_ref()?;
        let id = item.id();
        let index = conversation
            .items
            .iter()
            .position(|candidate| candidate.id() == id)?;
        let context = response(conversation, index)?;
        if context.latest
            && (self.working() || !self.approvals.is_empty() || !self.elicitation_panels.is_empty())
        {
            return None;
        }
        let user_item = context.user_item;
        let navigation = !self.composer_fills_thread();
        if context.text.is_empty() && !navigation {
            return None;
        }
        let group = SharedString::from(format!("message-actions-{id}"));
        Some(
            div()
                .id(group.clone())
                .test_support()
                .group(group.clone())
                .flex()
                .items_center()
                .justify_end()
                .gap_1()
                .pt_1()
                .when(!context.text.is_empty(), |actions| {
                    actions.child(
                        action_button(
                            SharedString::from(format!("copy-response-{id}")),
                            IconName::Copy,
                            cx,
                        )
                        .accessibility_label("复制本轮回复")
                        .tooltip("复制本轮回复")
                        .invisible()
                        .group_hover(group, |style| style.visible())
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(context.text.clone()));
                        }),
                    )
                })
                .when(navigation, |actions| {
                    actions.child(
                        action_button(
                            SharedString::from(format!("scroll-to-user-{id}")),
                            IconName::CornerLeftUp,
                            cx,
                        )
                        .accessibility_label("返回用户消息")
                        .tooltip("返回用户消息")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.scroll_to_response_user(user_item, cx);
                        })),
                    )
                })
                .into_any_element(),
        )
    }

    fn scroll_to_response_user(&mut self, user_item: Option<usize>, cx: &mut Context<Self>) {
        let row = self.conversation.as_ref().and_then(|conversation| {
            user_item
                .or_else(|| conversation.items.iter().rposition(is_user_message))
                .and_then(|index| conversation.row_for_item(index))
        });
        if let Some(item_ix) = row {
            self.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        } else {
            self.list.set_follow_mode(FollowMode::Tail);
        }
        cx.notify();
    }
}
