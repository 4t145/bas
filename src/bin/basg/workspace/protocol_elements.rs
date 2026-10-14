//! First-party presentation for ACP plans, notices, and context compaction.

use crate::conversation::{Compaction, ProtocolNotice, plan_id};
use bas::acp_v2 as acp2;
use gds_uim::el;

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
    crate::i18n::tr!(
        "plan-progress",
        completed = completed,
        total = entries.len()
    )
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
        let plans = &self.selected.timeline.conversation.as_ref()?.plans;
        if plans.is_empty() {
            return None;
        }
        let cards = plans.iter().map(|plan| self.render_plan_card(plan, cx));
        Some(
            el! {
                #chat::content_panel {
                    id: "session-plans";
                    test_support;
                    flex_shrink_0;
                    max_h: PLAN_PANEL_HEIGHT.px;
                    overflow_y_scroll; flex; flex_col; gap_2; py_2;
                } [..cards]
            }
            .into_any_element(),
        )
    }

    fn render_plan_card(&self, plan: &acp2::PlanUpdate, cx: &mut Context<Self>) -> AnyElement {
        let id = format!(
            "plan-{}",
            plan_id(plan).map(ToString::to_string).unwrap_or_default()
        );
        let mut panel = el! {
            #div {
                id: $SharedString::from(id.clone());
                test_support;
                rounded_md; border_1;
                border_color: $cx.theme().border;
                p_2; flex; flex_col; gap_1;
            }
        };
        match &plan.plan {
            acp2::PlanUpdateContent::Items(plan) => {
                panel = panel.child(el! {
                    #div { flex; items_center; gap_2; } [
                        Icon(IconName::ListTodo) {
                            size: crate::ui::ICON.px;
                            text_color: $cx.theme().muted_foreground;
                        },
                        #caption($plan_progress(&plan.entries), cx),
                    ]
                });
                for (index, entry) in plan.entries.iter().enumerate() {
                    let (label, icon, color) = match &entry.status {
                        acp2::PlanEntryStatus::Pending => (
                            crate::i18n::tr!("plan-pending").to_owned(),
                            IconName::Circle,
                            cx.theme().muted_foreground,
                        ),
                        acp2::PlanEntryStatus::InProgress => (
                            crate::i18n::tr!("status-running").to_owned(),
                            IconName::Loader,
                            cx.theme().primary,
                        ),
                        acp2::PlanEntryStatus::Completed => (
                            crate::i18n::tr!("status-completed").to_owned(),
                            IconName::Check,
                            cx.theme().success,
                        ),
                        acp2::PlanEntryStatus::Cancelled => (
                            crate::i18n::tr!("status-cancelled").to_owned(),
                            IconName::X,
                            cx.theme().muted_foreground,
                        ),
                        acp2::PlanEntryStatus::Other(status) => (
                            status.clone(),
                            IconName::CircleAlert,
                            cx.theme().muted_foreground,
                        ),
                        _ => (
                            crate::i18n::tr!("status-unknown").to_owned(),
                            IconName::CircleAlert,
                            cx.theme().muted_foreground,
                        ),
                    };
                    let priority = match &entry.priority {
                        acp2::PlanEntryPriority::High => {
                            crate::i18n::tr!("priority-high").to_owned()
                        }
                        acp2::PlanEntryPriority::Medium => {
                            crate::i18n::tr!("priority-medium").to_owned()
                        }
                        acp2::PlanEntryPriority::Low => crate::i18n::tr!("priority-low").to_owned(),
                        acp2::PlanEntryPriority::Other(priority) => priority.clone(),
                        _ => crate::i18n::tr!("priority-unknown").to_owned(),
                    };
                    panel = panel.child(el! {
                        #div {
                            id: $SharedString::from(format!("{id}-entry-{index}"));
                            test_support;
                            flex; items_start; gap_2;
                        } [
                            Button($SharedString::from(format!("{id}-status-{index}"))) {
                                ghost; xsmall;
                                tab_stop: false;
                                role: gpui_kit::Role::Status;
                                accessibility_label: $format!("{label} · {priority}");
                                tooltip: $format!("{label} · {priority}");
                            } [
                                $status::activity_icon(
                                    format!("{id}-status-icon-{index}"),
                                    icon,
                                    color,
                                    matches!(
                                        entry.status,
                                        acp2::PlanEntryStatus::InProgress
                                    ),
                                ),
                            ],
                            #div {
                                flex_1; min_w_0;
                                text_size: crate::ui::SMALL;
                            } [$entry.content.clone()],
                        ]
                    });
                }
            }
            acp2::PlanUpdateContent::Markdown(plan) => {
                panel = panel
                    .child(caption(crate::i18n::tr!("plan-title"), cx))
                    .child(self.markdown(
                        format!("{id}-markdown"),
                        plan.content.clone(),
                        PLAN_DOCUMENT_INDEX,
                        cx,
                    ));
            }
            acp2::PlanUpdateContent::File(plan) => {
                let uri = plan.uri.clone();
                panel = panel
                    .child(caption(crate::i18n::tr!("plan-file"), cx))
                    .child(el! {
                        Button($SharedString::from(format!("{id}-file"))) {
                            ghost; small; min_w_0; max_w_full;
                            icon: IconName::File;
                            label: $uri.clone();
                            tooltip: $crate::i18n::tr!("plan-open");
                            on_click: $move |_, _, cx| cx.open_url(&uri);
                        }
                    });
            }
            acp2::PlanUpdateContent::Other(plan) => {
                panel = panel.child(caption(
                    crate::i18n::tr!("plan-type", type = (plan.type_).to_string()),
                    cx,
                ));
            }
            _ => {
                panel = panel.child(caption(crate::i18n::tr!("plan-preview-unavailable"), cx));
            }
        }
        panel.into_any_element()
    }

    pub(super) fn render_notices(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notices = &self.selected.timeline.conversation.as_ref()?.notices;
        if notices.is_empty() {
            return None;
        }
        let notices = notices
            .iter()
            .map(|notice| self.render_protocol_notice(notice, cx));
        Some(
            el! {
                #chat::content_panel {
                    id: "protocol-notices";
                    test_support;
                    flex_shrink_0;
                    max_h: NOTICE_PANEL_HEIGHT.px;
                    overflow_y_scroll; flex; flex_col; gap_2; py_2;
                } [..notices]
            }
            .into_any_element(),
        )
    }

    fn render_protocol_notice(
        &self,
        notice: &ProtocolNotice,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = notice.id;
        let (label, icon, color) = match &notice.notice.severity {
            acp2::NoticeSeverity::Info => (
                crate::i18n::tr!("notice-info").to_owned(),
                IconName::Info,
                cx.theme().primary,
            ),
            acp2::NoticeSeverity::Warning => (
                crate::i18n::tr!("notice-warning").to_owned(),
                IconName::TriangleAlert,
                cx.theme().warning,
            ),
            acp2::NoticeSeverity::Error => (
                crate::i18n::tr!("notice-error").to_owned(),
                IconName::CircleX,
                cx.theme().danger,
            ),
            acp2::NoticeSeverity::Other(severity) => (
                severity.clone(),
                IconName::Info,
                cx.theme().muted_foreground,
            ),
            _ => (
                crate::i18n::tr!("notice-title").to_owned(),
                IconName::Info,
                cx.theme().muted_foreground,
            ),
        };
        el! {
            #div {
                id: $SharedString::from(format!("protocol-notice-{id}"));
                test_support;
                rounded_md; border_1;
                border_color: $color.opacity(0.35);
                bg: $color.opacity(0.05);
                p_2; flex; items_start; gap_2;
            } [
                Icon(icon) { small; text_color: color; },
                #div {
                    flex_1; min_w_0;
                    text_size: crate::ui::SMALL;
                } [
                    #div { text_color: color; } [
                        $format!("{label} · {}", notice.notice.title),
                    ],
                    ..$notice
                            .notice
                            .description
                            .as_ref()
                            .map(|description| el! { #div [$description.clone()] }),
                ],
                Button($SharedString::from(format!("dismiss-protocol-notice-{id}"))) {
                    ghost; xsmall;
                    icon: IconName::X;
                    accessibility_label: $crate::i18n::tr!("notice-dismiss");
                    tooltip: $crate::i18n::tr!("notice-dismiss");
                    on_click: $cx.listener(move |this, _, _, cx| {
                        this.dismiss_protocol_notice(id, cx);
                    });
                },
            ]
        }
        .into_any_element()
    }

    fn dismiss_protocol_notice(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(conversation) = &mut self.selected.timeline.conversation {
            conversation.notices.retain(|notice| notice.id != id);
        }
        cx.notify();
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
                crate::i18n::tr!("compaction-running").to_owned(),
                IconName::Loader,
                cx.theme().primary,
            ),
            acp2::CompactionStatus::Completed => (
                crate::i18n::tr!("compaction-completed").to_owned(),
                IconName::Check,
                cx.theme().success,
            ),
            acp2::CompactionStatus::Failed => (
                crate::i18n::tr!("compaction-failed").to_owned(),
                IconName::CircleX,
                cx.theme().danger,
            ),
            acp2::CompactionStatus::Cancelled => (
                crate::i18n::tr!("compaction-cancelled").to_owned(),
                IconName::X,
                cx.theme().muted_foreground,
            ),
            acp2::CompactionStatus::Other(status) => (
                crate::i18n::tr!("compaction-status", status = (status).to_string()),
                IconName::CircleAlert,
                cx.theme().muted_foreground,
            ),
            _ => (
                crate::i18n::tr!("compaction-title").to_owned(),
                IconName::CircleAlert,
                cx.theme().muted_foreground,
            ),
        };
        let expanded = self.selected.timeline.expanded.contains(id);
        let key = id.to_owned();
        let disclosure = if expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        let header = el! {
            Button($SharedString::from(format!("expand-{id}"))) {
                ghost; small; w_full;
                icon: disclosure;
                accessibility_label: $label.clone();
                on_prepaint: $self.track_content_header(id);
                on_click: $cx.listener(move |this, _, _, cx| {
                    this.toggle_content(&key, index, cx);
                });
            } [
                Icon(IconName::Archive) {
                    size: crate::ui::ICON.px;
                    text_color: $cx.theme().muted_foreground;
                },
                $status::activity_icon(
                        format!("compaction-status-{id}"),
                        icon,
                        color,
                        matches!(compaction.status, acp2::CompactionStatus::InProgress),
                ),
                #div { flex_1; text_color: color; } [$label],
            ]
        };
        el! {
            #div {
                rounded_md; border_1;
                border_color: $cx.theme().border;
                p_2; flex; flex_col; gap_2;
            } [
                $header,
                ..$compaction.error.as_ref().map(|error| el! {
                    #div {
                        text_size: crate::ui::SMALL;
                        text_color: $cx.theme().danger;
                    } [$error.clone()]
                }),
            ].gds_uim::m! {
                @expanded: $|panel| panel.children(
                    compaction_summary_blocks(&compaction.summary)
                        .iter()
                        .enumerate()
                        .map(
                            |(block_index, block)| match bas::model::presentation::content_to_v1(
                                block,
                            ) {
                                Ok(block) => self.render_block(
                                    format!("{id}-summary-{block_index}"),
                                    &block,
                                    index,
                                    cx,
                                ),
                                Err(_) => {
                                    caption(crate::i18n::tr!("summary-preview-unavailable"), cx)
                                        .into_any_element()
                                }
                            },
                        ),
                );
            }
        }
        .into_any_element()
    }
}

#[cfg(test)]
mod tests;
