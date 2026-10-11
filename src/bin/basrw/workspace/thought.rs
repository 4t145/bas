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
        if thoughts > 0 {
            summary.push(format!("{thoughts} 次思考"));
        }
        if tools > 0 {
            summary.push(format!("{tools} 次工具调用"));
        }
        if running > 0 {
            summary.push(format!("{running} 个执行中"));
        }
        if failed > 0 {
            summary.push(format!("{failed} 个失败"));
        }
        let expanded = self.expanded.contains(id);
        let key = id.to_owned();
        let color = if failed > 0 {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
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
                    .text_color(color)
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .accessibility_label(summary.join(" · "))
                    .on_prepaint(self.track_content_header(id))
                    .child(status::activity_icon(
                        SharedString::from(format!("activity-status-{id}")),
                        IconName::Lightbulb,
                        color,
                        activity_running,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(summary.join(" · ")),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        this.list.remeasure_items(index..index + 1);
                        cx.notify();
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
                div()
                    .id(SharedString::from(format!("expand-{id}")))
                    .test_support()
                    .on_prepaint(self.track_content_header(id))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
                    .child(status::activity_icon(
                        SharedString::from(format!("thought-status-{id}")),
                        IconName::Lightbulb,
                        cx.theme().muted_foreground,
                        streaming,
                    ))
                    .child(caption("思考", cx).flex_1())
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .small(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        this.list.remeasure_items(index..index + 1);
                        cx.notify();
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
