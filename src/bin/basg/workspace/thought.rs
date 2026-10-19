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
            return gds_uim::el! {
                #div { flex; flex_col; gap_1; } [
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
        let expanded = self.selected.timeline.expanded.contains(id);
        let label = summary
            .iter()
            .map(|(_, _, label, _)| label.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let summary = summary.into_iter().map(|(kind, icon, label, color)| {
            gds_uim::el! {
                #div {
                    id: $SharedString::from(format!("activity-{kind}-{id}"));
                    test_support;
                    role: $gpui_kit::Role::Label;
                    aria_label: $label.clone();
                    flex;
                    items_center;
                    gap_1;
                    text_size: $theme::content_font_size(cx);
                    line_height: $content_header::line_height(cx);
                    text_color: $color;
                } [
                    $status::activity_icon(
                        format!("activity-icon-{kind}-{id}"),
                        icon,
                        color,
                        activity_running
                            && (kind == "running" || (kind == "thoughts" && running == 0)),
                    ),
                    $label,
                ]
            }
        });
        let summary = gds_uim::el! {
            #div {
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
        gds_uim::el! {
            #content_header::frame(false, cx) [
                $header,
                ..$expanded.then(|| {
                    let entries = self.render_activity_entries(items, index, cx);
                    gds_uim::el! {
                        #div {
                            ml_2;
                            pl_2;
                            border_l_1;
                            border_color: $cx.theme().border;
                            flex;
                            flex_col;
                            pt_1;
                            gap_1;
                        } [..entries, $self.render_content_actions(id, id, index, cx)]
                    }
                }),
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
                gds_uim::el! {
                    #div {
                        id: $SharedString::from(item.id());
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
            .disclosure(
                id,
                crate::i18n::tr!("thought-title"),
                expanded,
                index,
                Some(leading),
                cx,
            )
            .text_color(cx.theme().muted_foreground);
        let header = self.content_header(id, disclosure, None);
        gds_uim::el! {
            #content_header::frame(false, cx) {
                text_color: $cx.theme().muted_foreground;
            } [
                $header,
                ..$expanded.then(|| {
                    let blocks = message.content.iter().enumerate().map(|(block_index, block)| {
                        self.render_block(format!("{id}-{block_index}"), block, index, cx)
                    }).collect::<Vec<_>>();
                    gds_uim::el! {
                        #div {
                            ml_1p5;
                            pl_3p5;
                            pt_1;
                            pb_3;
                            border_l_1;
                            border_color: $cx.theme().border;
                            flex;
                            flex_col;
                            gap_2;
                        } [..blocks, $self.render_content_actions(id, id, index, cx)]
                    }
                }),
            ]
        }
        .into_any_element()
    }
}
