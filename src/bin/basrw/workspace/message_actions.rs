use basr::model::Role;
use gpui_kit::ListOffset;

use super::*;

const ACTIONS_OPACITY: f32 = 0.4;

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
        Some(
            div()
                .id(SharedString::from(format!("message-actions-{id}")))
                .test_support()
                .flex()
                .items_center()
                .justify_end()
                .gap_1()
                .pt_1()
                .opacity(ACTIONS_OPACITY)
                .hover(|style| style.opacity(1.))
                .when(!context.text.is_empty(), |actions| {
                    actions.child(
                        layout::icon_button(
                            SharedString::from(format!("copy-response-{id}")),
                            IconName::Copy,
                        )
                        .accessibility_label("复制本轮回复")
                        .tooltip("复制本轮回复")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(context.text.clone()));
                        }),
                    )
                })
                .when(navigation, |actions| {
                    actions
                        .child(
                            layout::icon_button(
                                SharedString::from(format!("scroll-to-user-{id}")),
                                IconName::CornerLeftUp,
                            )
                            .accessibility_label("返回用户消息")
                            .tooltip("返回用户消息")
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.scroll_to_response_user(user_item, cx);
                                },
                            )),
                        )
                        .when(context.latest, |actions| {
                            actions.child(
                                layout::icon_button(
                                    SharedString::from(format!("scroll-to-top-{id}")),
                                    IconName::ArrowUp,
                                )
                                .accessibility_label("回到对话顶部")
                                .tooltip("回到对话顶部")
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.list.scroll_to(ListOffset::default());
                                        cx.notify();
                                    },
                                )),
                            )
                        })
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
