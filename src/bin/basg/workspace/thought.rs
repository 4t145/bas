use bas::model::{Message, Role, ToolCallStatus};

use super::*;

impl Workspace {
    pub(super) fn render_activity(
        &self,
        id: &str,
        items: &[Item],
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !cx.global::<theme::Appearance>().auto_collapse_activity {
            return el! {
                .div { flex; flex_col; gap_1; } [
                    ..$self.render_activity_entries(items, index, cx),
                ]
            }
            .into_any_element();
        }
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
                tr!("thought-count", thoughts = thoughts),
                muted,
            ));
        }
        if tools > 0 {
            summary.push((
                "tools",
                IconName::Wrench,
                tr!("tool-count", tools = tools),
                muted,
            ));
        }
        if running > 0 {
            summary.push((
                "running",
                IconName::Loader,
                tr!("running-count", running = running),
                cx.theme().primary,
            ));
        }
        if failed > 0 {
            summary.push((
                "failed",
                IconName::CircleX,
                tr!("failed-count", failed = failed),
                cx.theme().danger,
            ));
        }
        let expanded = self.selected.timeline.expanded.contains(id);
        let label = summary
            .iter()
            .map(|(_, _, label, _)| label.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let summary = summary.into_iter().map(|(kind, icon, label, color)| {
            el! {
                .div {
                    # format!("activity-{kind}-{id}").SharedString::from;
                    test_support;
                    role: gpui_kit::Role::Label;
                    aria_label: ~label;
                    flex;
                    items_center;
                    gap_1;
                    text_size: $theme::content_font_size(cx);
                    line_height: $content_header::line_height(cx);
                    text_color: color;
                } [
                    $status::activity_icon(
                        format!("activity-icon-{kind}-{id}"),
                        icon,
                        color,
                        activity_running
                            && (kind == "running" || (kind == "thoughts" && running == 0)),
                    ),
                    label,
                ]
            }
        });
        let summary = el! {
            .div {
                flex_1;
                min_w_0;
                flex;
                flex_wrap;
                gap_x_3;
                gap_y_1;
            } [..summary]
        }
        .into_any_element();
        let disclosure = self
            .disclosure_content(id, &label, expanded, index, summary, cx)
            .text_color(muted);
        let header = self.content_header(id, disclosure, None);
        el! {
            (false, cx).content_header::frame [
                header,
                @if (expanded)
                    .div {
                        ml_2;
                        pl_2;
                        border_l_1;
                        border_color: $cx.theme().border;
                        flex;
                        flex_col;
                        pt_1;
                        gap_1;
                    } [
                        ..$self.render_activity_entries(items, index, cx),
                        $self.render_content_actions(id, id, index, cx),
                    ],
                @end,
            ]
        }
        .into_any_element()
    }

    fn render_activity_entries(
        &self,
        items: &[Item],
        index: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        items
            .iter()
            .map(|item| {
                el! {
                    .div {
                        # $(item.id()).SharedString::from;
                        test_support;
                    } [$self.render_entry(item, index, cx)]
                }
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_thought(
        &self,
        id: &str,
        message: &Message,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.selected.timeline.expanded.contains(id);
        let streaming = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .is_some_and(|conversation| {
                conversation.items.iter().any(
                    |item| matches!(item, Item::Message { streaming: true, .. } if item.id() == id),
                )
            });
        let leading = status::activity_icon(
            SharedString::from(format!("thought-status-{id}")),
            IconName::Lightbulb,
            cx.theme().muted_foreground,
            streaming,
        );
        let disclosure = self
            .disclosure(id, tr!("thought-title"), expanded, index, Some(leading), cx)
            .text_color(cx.theme().muted_foreground);
        let header = self.content_header(id, disclosure, None);
        el! {
            (false, cx).content_header::frame {
                text_color: $cx.theme().muted_foreground;
            } [
                header,
                @if (expanded)
                    .div {
                        ml_1p5;
                        pl_3p5;
                        pt_1;
                        pb_3;
                        border_l_1;
                        border_color: $cx.theme().border;
                        flex;
                        flex_col;
                        gap_2;
                    } [
                        ..$message.content.iter().enumerate().map(|(block_index, block)| {
                            self.render_block(format!("{id}-{block_index}"), block, index, cx)
                        }).collect::<Vec<_>>(),
                        $self.render_content_actions(id, id, index, cx),
                    ],
                @end,
            ]
        }
        .into_any_element()
    }
}
