//! First-party presentation for ACP plans, notices, and context compaction.

use crate::conversation::{Compaction, plan_id};
use basr::acp_v2 as acp2;

use super::*;

const PLAN_PANEL_HEIGHT: f32 = 220.;
const NOTICE_PANEL_HEIGHT: f32 = 180.;
// Plan documents live outside the virtual transcript and do not need row remeasurement.
const PLAN_DOCUMENT_INDEX: usize = usize::MAX;

fn plan_progress(entries: &[acp2::PlanEntry]) -> String {
    let completed = entries
        .iter()
        .filter(|entry| entry.status == acp2::PlanEntryStatus::Completed)
        .count();
    format!("计划 · {completed}/{} 完成", entries.len())
}

fn compaction_summary_blocks(summary: &[acp2::ContentBlock]) -> Vec<acp2::ContentBlock> {
    let mut blocks: Vec<acp2::ContentBlock> = Vec::with_capacity(summary.len());
    for block in summary {
        if let acp2::ContentBlock::Text(chunk) = block
            && let Some(acp2::ContentBlock::Text(previous)) = blocks.last_mut()
            && previous.annotations == chunk.annotations
            && previous.meta == chunk.meta
        {
            previous.text.push_str(&chunk.text);
        } else {
            blocks.push(block.clone());
        }
    }
    blocks
}

impl Workspace {
    pub(super) fn render_plan(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let plans = &self.conversation.as_ref()?.plans;
        if plans.is_empty() {
            return None;
        }
        Some(
            chat::content_panel()
                .id("session-plans")
                .test_support()
                .flex_shrink_0()
                .max_h(px(PLAN_PANEL_HEIGHT))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .py_2()
                .children(plans.iter().map(|plan| {
                    let id = format!(
                        "plan-{}",
                        plan_id(plan).map(ToString::to_string).unwrap_or_default()
                    );
                    let mut panel = div()
                        .id(SharedString::from(id.clone()))
                        .test_support()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_1();
                    match &plan.plan {
                        acp2::PlanUpdateContent::Items(plan) => {
                            panel = panel.child(caption(plan_progress(&plan.entries), cx));
                            for (index, entry) in plan.entries.iter().enumerate() {
                                let (label, icon, color) = match &entry.status {
                                    acp2::PlanEntryStatus::Pending => (
                                        "待完成".to_owned(),
                                        IconName::Circle,
                                        cx.theme().muted_foreground,
                                    ),
                                    acp2::PlanEntryStatus::InProgress => {
                                        ("进行中".to_owned(), IconName::Loader, cx.theme().primary)
                                    }
                                    acp2::PlanEntryStatus::Completed => {
                                        ("已完成".to_owned(), IconName::Check, cx.theme().success)
                                    }
                                    acp2::PlanEntryStatus::Cancelled => (
                                        "已取消".to_owned(),
                                        IconName::X,
                                        cx.theme().muted_foreground,
                                    ),
                                    acp2::PlanEntryStatus::Other(status) => (
                                        status.clone(),
                                        IconName::CircleAlert,
                                        cx.theme().muted_foreground,
                                    ),
                                    _ => (
                                        "未知".to_owned(),
                                        IconName::CircleAlert,
                                        cx.theme().muted_foreground,
                                    ),
                                };
                                let priority = match &entry.priority {
                                    acp2::PlanEntryPriority::High => "高优先级".to_owned(),
                                    acp2::PlanEntryPriority::Medium => "中优先级".to_owned(),
                                    acp2::PlanEntryPriority::Low => "低优先级".to_owned(),
                                    acp2::PlanEntryPriority::Other(priority) => priority.clone(),
                                    _ => "未知优先级".to_owned(),
                                };
                                panel = panel.child(
                                    div()
                                        .id(SharedString::from(format!("{id}-entry-{index}")))
                                        .test_support()
                                        .flex()
                                        .items_start()
                                        .gap_2()
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "{id}-status-{index}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .tab_stop(false)
                                            .role(gpui_kit::Role::Status)
                                            .accessibility_label(format!("{label} · {priority}"))
                                            .tooltip(format!("{label} · {priority}"))
                                            .child(
                                                status::activity_icon(
                                                    format!("{id}-status-icon-{index}"),
                                                    icon,
                                                    color,
                                                    matches!(
                                                        entry.status,
                                                        acp2::PlanEntryStatus::InProgress
                                                    ),
                                                ),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_sm()
                                                .child(entry.content.clone()),
                                        ),
                                );
                            }
                        }
                        acp2::PlanUpdateContent::Markdown(plan) => {
                            panel = panel.child(caption("计划", cx)).child(self.markdown(
                                format!("{id}-markdown"),
                                plan.content.clone(),
                                PLAN_DOCUMENT_INDEX,
                                cx,
                            ));
                        }
                        acp2::PlanUpdateContent::File(plan) => {
                            let uri = plan.uri.clone();
                            panel = panel.child(caption("计划文件", cx)).child(
                                Button::new(SharedString::from(format!("{id}-file")))
                                    .ghost()
                                    .small()
                                    .min_w_0()
                                    .max_w_full()
                                    .icon(IconName::File)
                                    .label(uri.clone())
                                    .tooltip("打开计划文件")
                                    .on_click(move |_, _, cx| cx.open_url(&uri)),
                            );
                        }
                        acp2::PlanUpdateContent::Other(plan) => {
                            panel = panel.child(caption(format!("计划 · {}", plan.type_), cx));
                        }
                        _ => {
                            panel = panel.child(caption("计划内容暂不支持预览", cx));
                        }
                    }
                    panel
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_notices(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notices = &self.conversation.as_ref()?.notices;
        if notices.is_empty() {
            return None;
        }
        Some(
            chat::content_panel()
                .id("protocol-notices")
                .test_support()
                .flex_shrink_0()
                .max_h(px(NOTICE_PANEL_HEIGHT))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .py_2()
                .children(notices.iter().map(|notice| {
                    let id = notice.id;
                    let (label, icon, color) = match &notice.notice.severity {
                        acp2::NoticeSeverity::Info => {
                            ("信息".to_owned(), IconName::Info, cx.theme().primary)
                        }
                        acp2::NoticeSeverity::Warning => (
                            "警告".to_owned(),
                            IconName::TriangleAlert,
                            cx.theme().warning,
                        ),
                        acp2::NoticeSeverity::Error => {
                            ("错误".to_owned(), IconName::CircleX, cx.theme().danger)
                        }
                        acp2::NoticeSeverity::Other(severity) => (
                            severity.clone(),
                            IconName::Info,
                            cx.theme().muted_foreground,
                        ),
                        _ => (
                            "通知".to_owned(),
                            IconName::Info,
                            cx.theme().muted_foreground,
                        ),
                    };
                    div()
                        .id(SharedString::from(format!("protocol-notice-{id}")))
                        .test_support()
                        .rounded_md()
                        .border_1()
                        .border_color(color)
                        .p_2()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(Icon::new(icon).small().text_color(color))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .child(
                                    div()
                                        .text_color(color)
                                        .child(format!("{label} · {}", notice.notice.title)),
                                )
                                .children(
                                    notice
                                        .notice
                                        .description
                                        .as_ref()
                                        .map(|description| div().child(description.clone())),
                                ),
                        )
                        .child(
                            Button::new(SharedString::from(format!(
                                "dismiss-protocol-notice-{id}"
                            )))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .accessibility_label("关闭通知")
                            .tooltip("关闭通知")
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(conversation) = &mut this.conversation {
                                        conversation.notices.retain(|notice| notice.id != id);
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_compaction(
        &self,
        id: &str,
        compaction: &Compaction,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (label, icon, color) = match &compaction.status {
            acp2::CompactionStatus::InProgress => (
                "正在压缩上下文…".to_owned(),
                IconName::Loader,
                cx.theme().primary,
            ),
            acp2::CompactionStatus::Completed => (
                "上下文压缩完成".to_owned(),
                IconName::Check,
                cx.theme().success,
            ),
            acp2::CompactionStatus::Failed => (
                "上下文压缩失败".to_owned(),
                IconName::CircleX,
                cx.theme().danger,
            ),
            acp2::CompactionStatus::Cancelled => (
                "上下文压缩已取消".to_owned(),
                IconName::X,
                cx.theme().muted_foreground,
            ),
            acp2::CompactionStatus::Other(status) => (
                format!("上下文压缩 · {status}"),
                IconName::CircleAlert,
                cx.theme().muted_foreground,
            ),
            _ => (
                "上下文压缩".to_owned(),
                IconName::CircleAlert,
                cx.theme().muted_foreground,
            ),
        };
        let expanded = self.expanded.contains(id);
        let key = id.to_owned();
        div()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                Button::new(SharedString::from(format!("expand-{id}")))
                    .ghost()
                    .small()
                    .w_full()
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .accessibility_label(label.clone())
                    .child(status::activity_icon(
                        format!("compaction-status-{id}"),
                        icon,
                        color,
                        matches!(compaction.status, acp2::CompactionStatus::InProgress),
                    ))
                    .child(div().flex_1().text_color(color).child(label))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        this.list.remeasure_items(index..index + 1);
                        cx.notify();
                    })),
            )
            .children(compaction.error.as_ref().map(|error| {
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
            }))
            .when(expanded, |panel| {
                panel.children(
                    compaction_summary_blocks(&compaction.summary)
                        .iter()
                        .enumerate()
                        .map(
                            |(block_index, block)| match basr::model::presentation::content_to_v1(
                                block,
                            ) {
                                Ok(block) => self.render_block(
                                    format!("{id}-summary-{block_index}"),
                                    &block,
                                    index,
                                    cx,
                                ),
                                Err(_) => caption("摘要内容暂不支持预览", cx).into_any_element(),
                            },
                        ),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
