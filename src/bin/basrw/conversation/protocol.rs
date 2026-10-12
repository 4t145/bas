//! Protocol presentation state is reconstructed from the same events as the transcript.

use super::*;

const LEGACY_PLAN_ID: &str = "_basr_legacy_plan";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProtocolNotice {
    pub id: u64,
    pub notice: acp2::Notice,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Compaction {
    pub id: acp2::CompactionId,
    pub status: acp2::CompactionStatus,
    pub summary: Vec<acp2::ContentBlock>,
    pub error: Option<String>,
    pub meta: Option<acp2::Meta>,
}

impl Compaction {
    fn new(id: acp2::CompactionId) -> Self {
        Self {
            id,
            status: acp2::CompactionStatus::InProgress,
            summary: Vec::new(),
            error: None,
            meta: None,
        }
    }

    fn update(&mut self, update: &acp2::CompactionUpdate) {
        self.status = update.status.clone();
        if !update.summary.is_undefined() {
            self.summary = update.summary.clone().take().unwrap_or_default();
        }
        if !update.error.is_undefined() {
            self.error = update.error.clone().take();
        }
        if !update.meta.is_undefined() {
            self.meta = update.meta.clone().take();
        }
    }
}

pub(crate) fn plan_id(plan: &acp2::PlanUpdate) -> Option<&acp2::PlanId> {
    match &plan.plan {
        acp2::PlanUpdateContent::Items(plan) => Some(&plan.plan_id),
        acp2::PlanUpdateContent::Markdown(plan) => Some(&plan.plan_id),
        acp2::PlanUpdateContent::File(plan) => Some(&plan.plan_id),
        acp2::PlanUpdateContent::Other(plan) => Some(&plan.plan_id),
        _ => None,
    }
}

impl Conversation {
    /// Applies the common v2 presentation forms after transport normalization.
    pub(crate) fn apply_protocol_update(&mut self, update: &acp2::SessionUpdate) -> Option<usize> {
        match update {
            acp2::SessionUpdate::StateUpdate(state) => {
                self.foreground_state = Some(state.clone());
                None
            }
            acp2::SessionUpdate::TerminalUpdate(update) => {
                let terminal = self
                    .terminals
                    .entry(update.terminal_id.to_string())
                    .or_default();
                if let Err(error) = terminal.apply_update(update) {
                    return self.notice(crate::i18n::tr!(
                        "terminal-invalid-output",
                        error = (error).to_string()
                    ));
                }
                self.terminal_item(&update.terminal_id)
            }
            acp2::SessionUpdate::TerminalOutputChunk(chunk) => {
                let terminal = self
                    .terminals
                    .entry(chunk.terminal_id.to_string())
                    .or_default();
                if let Err(error) = terminal.push_chunk(chunk) {
                    return self.notice(crate::i18n::tr!(
                        "terminal-invalid-output",
                        error = (error).to_string()
                    ));
                }
                self.terminal_item(&chunk.terminal_id)
            }
            acp2::SessionUpdate::PlanUpdate(update) => {
                if let Some(id) = plan_id(update) {
                    if let Some(plan) = self.plans.iter_mut().find(|plan| plan_id(plan) == Some(id))
                    {
                        *plan = update.clone();
                    } else {
                        self.plans.push(update.clone());
                    }
                }
                None
            }
            acp2::SessionUpdate::PlanRemoved(removed) => {
                self.plans
                    .retain(|plan| plan_id(plan) != Some(&removed.plan_id));
                None
            }
            acp2::SessionUpdate::UsageUpdate(usage) => {
                self.usage = Some(usage.clone());
                None
            }
            acp2::SessionUpdate::Notice(notice) => {
                self.push_notice(notice.clone());
                None
            }
            acp2::SessionUpdate::CompactionUpdate(update) => {
                let (index, changed) = self.compaction_index(&update.compaction_id);
                if let Item::Compaction(compaction) = &mut self.items[index] {
                    compaction.update(update);
                }
                Some(changed.unwrap_or(index))
            }
            acp2::SessionUpdate::CompactionSummaryChunk(chunk) => {
                let (index, changed) = self.compaction_index(&chunk.compaction_id);
                if let Item::Compaction(compaction) = &mut self.items[index] {
                    compaction.summary.push(chunk.content.clone());
                }
                Some(changed.unwrap_or(index))
            }
            _ => None,
        }
    }

    /// Receives transient notices separately from replayable transcript events.
    pub(crate) fn push_notice(&mut self, notice: acp2::Notice) {
        self.next_notice += 1;
        self.notices.push(ProtocolNotice {
            id: self.next_notice,
            notice,
        });
    }

    fn terminal_item(&self, id: &acp2::TerminalId) -> Option<usize> {
        self.items.iter().position(|item| matches!(item, Item::Tool(tool) if tool.content.iter().any(|content|
            matches!(content, acp::ToolCallContent::Terminal(terminal) if terminal.terminal_id.to_string() == id.to_string())
        )))
    }

    fn compaction_index(&mut self, id: &acp2::CompactionId) -> (usize, Option<usize>) {
        if let Some(index) = self
            .items
            .iter()
            .position(|item| matches!(item, Item::Compaction(compaction) if &compaction.id == id))
        {
            (index, None)
        } else {
            let changed = self.seal();
            self.items
                .push(Item::Compaction(Compaction::new(id.clone())));
            (self.items.len() - 1, changed)
        }
    }

    pub(super) fn apply_v1_protocol(&mut self, update: acp::SessionUpdate) -> Option<usize> {
        // These v1 extensions share their wire representation with v2.
        let converted = serde_json::to_value(update).and_then(serde_json::from_value);
        match converted {
            Ok(update) => self.apply_protocol_update(&update),
            Err(error) => {
                tracing::warn!(%error, "cannot normalize protocol presentation update");
                self.notice(crate::i18n::tr!("protocol-update-invalid"))
            }
        }
    }

    pub(super) fn apply_legacy_plan(&mut self, plan: &acp::Plan) {
        let entries = serde_json::to_value(&plan.entries).and_then(serde_json::from_value);
        if let Ok(entries) = entries {
            self.apply_protocol_update(&acp2::SessionUpdate::PlanUpdate(acp2::PlanUpdate::new(
                acp2::PlanUpdateContent::items(LEGACY_PLAN_ID, entries),
            )));
        }
    }
}
