use basr::model::{Message, Role, ToolCallStatus};

use super::*;

impl Workspace {
    pub(super) fn render_activity(
        &self,
        id: &str,
        items: &[Item],
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut thoughts = 0;
        let mut tools = 0;
        let mut running = 0;
        let mut failed = 0;
        let mut activity_running = false;
        for item in items {
            match item {
                Item::Message { message, streaming } if message.role == Role::Thought => {
                    thoughts += 1;
                    activity_running |= *streaming;
                }
                Item::Tool(tool) => {
                    tools += 1;
                    activity_running |= tool.status == ToolCallStatus::InProgress;
                    match tool.status {
                        ToolCallStatus::Pending | ToolCallStatus::InProgress => running += 1,
                        ToolCallStatus::Failed => failed += 1,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let mut summary = Vec::new();
        let muted = cx.theme().muted_foreground;
        if thoughts > 0 {
            summary.push((
                "thoughts",
                IconName::Lightbulb,
                crate::i18n::tr!("thought-count", thoughts = thoughts),
                muted,
            ));
        }
        if tools > 0 {
            summary.push((
                "tools",
                IconName::Wrench,
                crate::i18n::tr!("tool-count", tools = tools),
                muted,
            ));
        }
        if running > 0 {
            summary.push((
                "running",
                IconName::Loader,
                crate::i18n::tr!("running-count", running = running),
                cx.theme().primary,
            ));
        }
        if failed > 0 {
            summary.push((
                "failed",
                IconName::CircleX,
                crate::i18n::tr!("failed-count", failed = failed),
                cx.theme().danger,
            ));
        }
        let expanded = self.expanded.contains(id);
        let key = id.to_owned();
        let label = summary
            .iter()
            .map(|(_, _, label, _)| label.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_2()
            .py_1()
            .child(
                Button::new(SharedString::from(format!("expand-{id}")))
                    .ghost()
                    .small()
                    .w_full()
                    .h_auto()
                    .min_h(px(crate::ui::CONTROL))
                    .py_1()
                    .text_color(muted)
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .accessibility_label(label)
                    .on_prepaint(self.track_content_header(id))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .gap_x_3()
                            .gap_y_1()
                            .children(summary.into_iter().map(|(kind, icon, label, color)| {
                                div()
                                    .id(SharedString::from(format!("activity-{kind}-{id}")))
                                    .test_support()
                                    .role(gpui_kit::Role::Label)
                                    .aria_label(label.clone())
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_size(crate::ui::SMALL)
                                    .text_color(color)
                                    .child(status::activity_icon(
                                        format!("activity-icon-{kind}-{id}"),
                                        icon,
                                        color,
                                        activity_running
                                            && (kind == "running"
                                                || (kind == "thoughts" && running == 0)),
                                    ))
                                    .child(label)
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_content(&key, index, cx);
                    })),
            )
            .when(expanded, |group| {
                group.child(
                    div()
                        .ml_2()
                        .pl_2()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(items.iter().map(|item| {
                            div()
                                .id(SharedString::from(item.id()))
                                .test_support()
                                .child(self.render_entry(item, index, cx))
                        }))
                        .child(self.render_content_actions(id, id, index, cx)),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_thought(
        &self,
        id: &str,
        message: &Message,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded.contains(id);
        let key = id.to_owned();
        let streaming = self.conversation.as_ref().is_some_and(|conversation| {
            conversation.items.iter().any(
                |item| matches!(item, Item::Message { streaming: true, .. } if item.id() == id),
            )
        });
        div()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .text_color(cx.theme().muted_foreground)
            .child(
                Button::new(SharedString::from(format!("expand-{id}")))
                    .ghost()
                    .small()
                    .w_full()
                    .h(px(crate::ui::CONTROL))
                    .text_color(cx.theme().muted_foreground)
                    .accessibility_label(crate::i18n::tr!("thought-title"))
                    .on_prepaint(self.track_content_header(id))
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .child(status::activity_icon(
                        SharedString::from(format!("thought-status-{id}")),
                        IconName::Lightbulb,
                        cx.theme().muted_foreground,
                        streaming,
                    ))
                    .child(caption(crate::i18n::tr!("thought-title"), cx).flex_1())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_content(&key, index, cx);
                    })),
            )
            .when(expanded, |panel| {
                panel.child(
                    div()
                        .ml_1p5()
                        .pl_3p5()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(
                            message
                                .content
                                .iter()
                                .enumerate()
                                .map(|(block_index, block)| {
                                    self.render_block(
                                        format!("{id}-{block_index}"),
                                        block,
                                        index,
                                        cx,
                                    )
                                }),
                        )
                        .child(self.render_content_actions(id, id, index, cx)),
                )
            })
            .into_any_element()
    }
}
