use bas::model::Role;
use gpui_kit::{Bounds, ListOffset};

use super::*;

pub(super) fn action_button(
    id: impl Into<gpui_kit::ElementId>,
    icon: IconName,
    cx: &App,
) -> Button {
    ui::subtle_icon_button(id, icon, cx)
}

impl Workspace {
    pub(super) fn toggle_content(&mut self, key: &str, index: usize, cx: &mut Context<Self>) {
        self.selected.timeline.list.pause_following_tail();
        if !self.selected.timeline.expanded.remove(key) {
            self.selected.timeline.expanded.insert(key.to_owned());
        }
        self.selected
            .timeline
            .list
            .remeasure_items(index..index + 1);
        cx.notify();
    }

    pub(super) fn track_content_header(
        &self,
        id: &str,
    ) -> impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static {
        let bounds = self.selected.timeline.bounds.clone();
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
        let copy = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .and_then(|conversation| {
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
        el! {
            .div {
                # ~group;
                test_support;
                group: ~group;
                flex;
                items_center;
                justify_end;
                gap_1;
                pt_1;
            } [
                @if (let Some(copy) = copy)
                    (format!("copy-{id}").SharedString::from, IconName::Copy, cx).action_button {
                        accessibility_label: tr!("action-copy-thought");
                        tooltip: tr!("action-copy-thought");
                        invisible;
                        group_hover: group, { visible; };
                        on_click: move (_, _, cx) => ($&copy, cx).Self::on_content_copy;
                    },
                @end,
                (
                    format!("scroll-to-content-{id}").SharedString::from,
                    IconName::CornerLeftUp,
                    cx,
                ).action_button {
                    accessibility_label: tr!("action-content-top");
                    tooltip: tr!("action-content-top");
                    on_click: $cx.listener(move |this, _, _, cx| {
                        this.on_content_top(&top_id, index, cx);
                    });
                },
                (
                    format!("collapse-content-{id}").SharedString::from,
                    IconName::ChevronUp,
                    cx,
                ).action_button {
                    accessibility_label: tr!("action-collapse-content");
                    tooltip: tr!("action-collapse-content");
                    on_click: $cx.listener(move |this, _, _, cx| {
                        this.on_content_collapse(&collapse_id, &collapse_key, index, cx);
                    });
                },
            ]
        }
        .into_any_element()
    }

    fn on_content_copy(copy: &str, cx: &mut App) {
        cx.stop_propagation();
        cx.write_to_clipboard(ClipboardItem::new_string(copy.to_owned()));
    }

    fn on_content_top(&mut self, id: &str, index: usize, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let offset = self.content_scroll_offset(id, index);
        self.selected.timeline.list.scroll_to(offset);
        cx.notify();
    }

    fn on_content_collapse(&mut self, id: &str, key: &str, index: usize, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let offset = self.content_scroll_offset(id, index);
        self.selected.timeline.expanded.remove(key);
        self.selected
            .timeline
            .list
            .remeasure_items(index..index + 1);
        self.selected.timeline.list.scroll_to(offset);
        cx.notify();
    }

    pub(super) fn content_scroll_offset(&self, id: &str, index: usize) -> ListOffset {
        let offset_in_item = self
            .selected
            .timeline
            .bounds
            .borrow()
            .get(id)
            .zip(self.selected.timeline.list.bounds_for_item(index))
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
        let conversation = self.selected.timeline.conversation.as_ref()?;
        let id = item.id();
        let index = conversation
            .items
            .iter()
            .position(|candidate| candidate.id() == id)?;
        let context = conversation.response(index)?;
        if context.latest
            && (self.working()
                || !self.selected.approval.panel.is_empty()
                || !self.selected.elicitations.is_empty())
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
            el! {
                .div {
                    # ~group;
                    test_support;
                    group: ~group;
                    flex;
                    items_center;
                    justify_end;
                    gap_1;
                    pt_1;
                } [
                    @if (!context.text.is_empty())
                        (
                            format!("copy-response-{id}").SharedString::from,
                            IconName::Copy,
                            cx,
                        ).action_button {
                            accessibility_label: tr!("action-copy-response");
                            tooltip: tr!("action-copy-response");
                            invisible;
                            group_hover: group, { visible; };
                            on_click: move (_, _, cx) => ($&context.text, cx).Self::on_response_copy;
                        },
                    @end,
                    @if (navigation)
                        (
                            format!("scroll-to-user-{id}").SharedString::from,
                            IconName::CornerLeftUp,
                            cx,
                        ).action_button {
                            accessibility_label: tr!("action-user-message");
                            tooltip: tr!("action-user-message");
                            on_click: $cx.listener(move |this, _, _, cx| {
                                this.scroll_to_response_user(user_item, cx);
                            });
                        },
                    @end,
                ]
            }
            .into_any_element(),
        )
    }

    fn on_response_copy(text: &str, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
    }

    fn scroll_to_response_user(&mut self, user_item: Option<usize>, cx: &mut Context<Self>) {
        let row = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .and_then(|conversation| {
                // A greeting has no associated user; its navigation shortcut targets the latest user.
                user_item
                    .or_else(|| {
                        conversation.items.iter().rposition(|item| {
                            matches!(item, Item::Message { message, .. } if message.role == Role::User)
                        })
                    })
                    .and_then(|index| conversation.row_for_item(index))
            });
        if let Some(item_ix) = row {
            self.selected.timeline.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        } else {
            self.selected
                .timeline
                .list
                .set_follow_mode(FollowMode::Tail);
        }
        cx.notify();
    }
}
