//! First-party presentation for ACP plans, notices, and context compaction.

use crate::conversation::{Compaction, ProtocolNotice};
use bas::acp_v2 as acp2;
use gds_uim::el;
use gpui_kit::component::badge::Badge;

use super::*;

const NOTICE_PANEL_HEIGHT: f32 = 180.;
const PLAN_PREVIEW_CHARACTERS: usize = 320;

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

fn unfinished(entry: &acp2::PlanEntry) -> bool {
    !matches!(
        entry.status,
        acp2::PlanEntryStatus::Completed | acp2::PlanEntryStatus::Cancelled
    )
}

fn plan_remaining(plans: &[acp2::PlanUpdate]) -> usize {
    plans
        .iter()
        .map(|plan| match &plan.plan {
            acp2::PlanUpdateContent::Items(items) => items
                .entries
                .iter()
                .filter(|entry| unfinished(entry))
                .count(),
            _ => 0,
        })
        .sum()
}

fn plan_has_entries(plans: &[acp2::PlanUpdate]) -> bool {
    plans.iter().any(|plan| {
        matches!(&plan.plan, acp2::PlanUpdateContent::Items(items) if !items.entries.is_empty())
    })
}

fn truncated(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let preview: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

#[derive(Clone)]
enum PlanHintRow {
    Heading(String),
    Preview(String),
    Pending(String),
    InProgress(String),
    Other(String),
}

fn plan_hint_rows(plans: &[acp2::PlanUpdate]) -> Vec<PlanHintRow> {
    let mut rows = Vec::new();
    for plan in plans {
        match &plan.plan {
            acp2::PlanUpdateContent::Items(items) => {
                rows.push(PlanHintRow::Heading(plan_progress(&items.entries)));
                rows.extend(
                    items
                        .entries
                        .iter()
                        .filter(|entry| unfinished(entry))
                        .map(|entry| match entry.status {
                            acp2::PlanEntryStatus::InProgress => {
                                PlanHintRow::InProgress(entry.content.clone())
                            }
                            acp2::PlanEntryStatus::Pending => {
                                PlanHintRow::Pending(entry.content.clone())
                            }
                            _ => PlanHintRow::Other(entry.content.clone()),
                        }),
                );
            }
            acp2::PlanUpdateContent::Markdown(plan) => {
                rows.push(PlanHintRow::Heading(
                    crate::i18n::tr!("plan-title").to_string(),
                ));
                rows.push(PlanHintRow::Preview(truncated(
                    &plan.content,
                    PLAN_PREVIEW_CHARACTERS,
                )));
            }
            acp2::PlanUpdateContent::File(plan) => {
                rows.push(PlanHintRow::Heading(
                    crate::i18n::tr!("plan-file").to_string(),
                ));
                rows.push(PlanHintRow::Preview(plan.uri.clone()));
            }
            acp2::PlanUpdateContent::Other(plan) => {
                rows.push(PlanHintRow::Heading(crate::i18n::tr!(
                    "plan-type",
                    type = (plan.type_).to_string()
                )));
            }
            _ => {
                rows.push(PlanHintRow::Heading(
                    crate::i18n::tr!("plan-preview-unavailable").to_string(),
                ));
            }
        }
    }
    rows
}

struct PlanHint {
    label: String,
    content: AnyElement,
}

fn plan_hint_content(rows: &[PlanHintRow], cx: &App) -> AnyElement {
    el! {
        #div { flex; flex_col; gap_1; min_w_0; } [..$rows.iter().enumerate().map(|(index, row)| {
            let PlanHint { label, content } = plan_hint_row(index, row, cx);
            el! {
                #div {
                    id: $SharedString::from(format!("plan-hint-row-{index}"));
                    test_support; role: gpui_kit::Role::Label;
                    aria_label: $label;
                    flex; items_start; gap_1p5; min_w_0;
                } [$content]
            }
        })]
    }
    .into_any_element()
}

fn plan_hint_row(index: usize, row: &PlanHintRow, cx: &App) -> PlanHint {
    let line_height = crate::ui::SMALL * 1.4;
    let (marker, text, heading) = match row {
        PlanHintRow::Heading(text) => (None, text.as_str(), true),
        PlanHintRow::Preview(text) => (None, text.as_str(), false),
        PlanHintRow::Pending(text) => (
            Some((IconName::Circle, cx.theme().muted_foreground)),
            text.as_str(),
            false,
        ),
        PlanHintRow::InProgress(text) => (
            Some((IconName::Loader, cx.theme().primary)),
            text.as_str(),
            false,
        ),
        PlanHintRow::Other(text) => (
            Some((IconName::CircleAlert, cx.theme().muted_foreground)),
            text.as_str(),
            false,
        ),
    };
    let content = el! {
        #div {
            flex_1; min_w_0; whitespace_normal;
            line_height: line_height;
            @$heading => {
                font_weight: FontWeight::SEMIBOLD;
                text_color: $cx.theme().muted_foreground;
            };
        } [$crate::ui::tooltip::wrap_tokens(text)]
    }
    .into_any_element();
    let content = match marker {
        Some((icon, color)) => el! {
            #div { flex; items_start; gap_1p5; min_w_0; } [
                #div {
                    id: $SharedString::from(format!("plan-hint-marker-{index}")); test_support;
                    w: crate::ui::ICON.px; h: line_height;
                    flex_shrink_0; flex; items_center; justify_center;
                } [
                    $status::activity_icon(
                        SharedString::from(format!("plan-hint-status-{index}")),
                        icon, color, matches!(row, PlanHintRow::InProgress(_)),
                    ),
                ],
                $content,
            ]
        }
        .into_any_element(),
        None => content,
    };
    PlanHint {
        label: text.to_owned(),
        content,
    }
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
    pub(super) fn render_plan_indicator(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let plans = &self.selected.timeline.conversation.as_ref()?.plans;
        if plans.is_empty() {
            return None;
        }
        let remaining = plan_remaining(plans);
        let hint = if remaining > 0 {
            crate::i18n::tr!("plan-indicator", count = remaining).to_string()
        } else if plan_has_entries(plans) {
            crate::i18n::tr!("plan-complete").to_string()
        } else {
            crate::i18n::tr!("plan-title").to_string()
        };
        let rows = plan_hint_rows(plans);
        Some(
            el! {
                #div {
                    id: "session-plan-indicator";
                    test_support;
                    relative; flex; flex_shrink_0;
                    role: gpui_kit::Role::Status;
                    aria_label: $hint.clone();
                    @$remaining > 0 => { pr_3; };
                    hoverable_tooltip: $move |window, cx| {
                        let rows = rows.clone();
                        crate::ui::tooltip::element(
                            "plan-hint",
                            hint.clone(),
                            move |_, cx| plan_hint_content(&rows, cx),
                            window,
                            cx,
                        )
                    };
                } [
                    Badge {
                        count: remaining; small; max: 99;
                        color: $cx.theme().primary;
                    } [
                        #div {
                            id: "session-plan-icon"; test_support;
                            size: layout::ICON_BUTTON_SIZE.px;
                            flex; items_center; justify_center;
                            text_color: $cx.theme().muted_foreground;
                        } [
                            Icon(IconName::ListTodo) { size: crate::ui::ICON.px; },
                        ],
                    ],
                ]
            }
            .into_any_element(),
        )
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
        let message = match &notice.notice.description {
            Some(description) => format!("{}\n{description}", notice.notice.title),
            None => notice.notice.title.clone(),
        };
        el! {
            #div {
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
                        $label,
                    ],
                    $crate::ui::feedback(format!("protocol-notice-{id}"), message, color),
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
        let status = el! {
                Button($SharedString::from(format!("status-{id}"))) {
                    ghost; xsmall; cursor_default;
                    w: crate::ui::COMPACT_CONTROL.px; h: crate::ui::COMPACT_CONTROL.px;
                    p_0; flex_none; tooltip: $label;
                } [
                    $status::activity_icon(
                        format!("compaction-status-{id}"),
                        icon,
                        color,
                        matches!(compaction.status, acp2::CompactionStatus::InProgress),
                    ),
                ],
        }
        .into_any_element();
        let leading = status::activity_icon(
            format!("compaction-kind-{id}"),
            IconName::Archive,
            cx.theme().muted_foreground,
            false,
        );
        let disclosure = self.disclosure(
            id,
            crate::i18n::tr!("compaction-title"),
            expanded,
            index,
            Some(leading),
            cx,
        );
        let header = self.content_header(id, disclosure, Some(status));
        el! {
            #content_header::frame(true, cx) [
                $header,
                ..$compaction.error.as_ref().map(|error| el! {
                    #div {
                        px_3; pb_3;
                    } [$crate::ui::feedback(format!("compaction-error-{id}"), error.clone(), cx.theme().danger)]
                }),
            ].gds_uim::m! {
                @expanded: $|panel| panel.child(el! {
                    #div { flex; flex_col; gap_3; px_3; pt_1; pb_3; } [..$
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
                        )
                    ]
                });
            }
        }
        .into_any_element()
    }
}

#[cfg(test)]
mod tests;
