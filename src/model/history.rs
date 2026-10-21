//! Payload-free replay dependencies shared by storage and live consumers.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{Cursor, Event, EventKind};
use crate::acp;

#[cfg(test)]
mod tests;

/// Derived state only: identities survive turn boundaries.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DependencyIndex {
    anchors: HashMap<String, Anchor>,
    legacy: Option<(u8, u64)>,
    incomplete: bool,
    turn_start: Option<u64>,
    interruption: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Anchor {
    first: u64,
    last: u64,
}

#[derive(Clone, Debug)]
pub struct Dependency {
    pub floor: Cursor,
    pub context_key: Option<String>,
    /// First insertion of the active plan generation, or the usage snapshot.
    /// `None` with a context key denotes removal.
    pub context_start: Option<Cursor>,
}

impl DependencyIndex {
    /// Discards identities not present in retained history. Unknown identities
    /// subsequently depend on START: they may refer to an evicted item.
    pub fn evict_through(&mut self, cursor: Cursor) {
        self.incomplete = cursor != Cursor::START;
        self.anchors.retain(|_, anchor| anchor.last > cursor.raw());
    }

    /// On a partial replay, an unknown identity may address unloaded history.
    /// Call before observing a live event; `true` requests a fresh storage window.
    pub fn may_reference_unloaded(&self, event: &Event) -> bool {
        self.reference_floor(event, 0) == 0
    }

    /// Whether an event refers to an identity already encountered by this index.
    pub fn references_history(&self, event: &Event) -> bool {
        self.reference_floor(event, event.cursor.raw()) < event.cursor.raw()
    }

    // Look up only this event's identities, never clone the session-sized index
    // on the streaming path. Missing identities have caller-specific semantics.
    fn reference_floor(&self, event: &Event, missing: u64) -> u64 {
        let mut floor = event.cursor.raw();
        let mut reference = |key: String| {
            floor = floor.min(
                self.anchors
                    .get(&key)
                    .map_or(missing, |anchor| anchor.first),
            );
        };
        match &event.kind {
            EventKind::Acp(update) => match update {
                acp::SessionUpdate::UserMessageChunk(chunk)
                | acp::SessionUpdate::AgentMessageChunk(chunk)
                | acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                    let role = match update {
                        acp::SessionUpdate::UserMessageChunk(_) => 0,
                        acp::SessionUpdate::AgentMessageChunk(_) => 1,
                        _ => 2,
                    };
                    if let Some(id) = &chunk.message_id {
                        reference(format!("message:{role}:{id}"));
                    } else if let Some((previous, start)) = self.legacy
                        && previous == role
                    {
                        floor = floor.min(start);
                    }
                }
                acp::SessionUpdate::ToolCall(tool) => {
                    reference(format!("tool:{}", tool.tool_call_id));
                    terminal_references(tool.meta.as_ref(), &tool.content, &mut reference);
                }
                acp::SessionUpdate::ToolCallUpdate(tool) => {
                    reference(format!("tool:{}", tool.tool_call_id));
                    terminal_references(
                        tool.meta.as_ref(),
                        tool.fields.content.as_deref().unwrap_or_default(),
                        &mut reference,
                    );
                }
                acp::SessionUpdate::CompactionUpdate(update) => {
                    reference(format!("compaction:{}", update.compaction_id));
                }
                acp::SessionUpdate::CompactionSummaryChunk(chunk) => {
                    reference(format!("compaction:{}", chunk.compaction_id));
                }
                _ => {}
            },
            EventKind::TurnFinished { .. } => floor = self.turn_start.unwrap_or(missing),
            EventKind::Failed { .. } => floor = self.interruption.unwrap_or(missing),
            _ => {}
        }
        floor
    }

    pub fn observe(&mut self, event: &Event) -> Dependency {
        let cursor = event.cursor.raw();
        let mut floor = cursor;
        let mut context_key = None;
        let mut removed = false;
        match &event.kind {
            EventKind::Acp(update) => {
                let chunk = match update {
                    acp::SessionUpdate::UserMessageChunk(c) => Some((0, c)),
                    acp::SessionUpdate::AgentMessageChunk(c) => Some((1, c)),
                    acp::SessionUpdate::AgentThoughtChunk(c) => Some((2, c)),
                    _ => None,
                };
                if let Some((role, chunk)) = chunk {
                    if let Some(id) = &chunk.message_id {
                        let key = format!("message:{role}:{id}");
                        floor = self.identity(key, cursor);
                        self.legacy = None;
                    } else {
                        if let Some((previous_role, start)) = self.legacy
                            && previous_role == role
                        {
                            floor = start;
                        }
                        self.legacy = Some((role, floor));
                    }
                } else {
                    match update {
                        acp::SessionUpdate::ToolCall(tool) => {
                            floor = self.identity(format!("tool:{}", tool.tool_call_id), cursor);
                            terminal_references(tool.meta.as_ref(), &tool.content, &mut |key| {
                                self.reference(key, cursor, &mut floor)
                            });
                        }
                        acp::SessionUpdate::ToolCallUpdate(tool) => {
                            floor = self.identity(format!("tool:{}", tool.tool_call_id), cursor);
                            terminal_references(
                                tool.meta.as_ref(),
                                tool.fields.content.as_deref().unwrap_or_default(),
                                &mut |key| self.reference(key, cursor, &mut floor),
                            );
                        }
                        acp::SessionUpdate::Plan(plan) => {
                            context_key = Some("plan:_bas_legacy_plan".into());
                            removed = plan.entries.is_empty();
                        }
                        acp::SessionUpdate::UsageUpdate(_) => context_key = Some("usage".into()),
                        acp::SessionUpdate::PlanUpdate(update) => {
                            let id = match &update.plan {
                                acp::PlanUpdateContent::Items(plan) => Some(&plan.plan_id),
                                acp::PlanUpdateContent::Markdown(plan) => Some(&plan.plan_id),
                                acp::PlanUpdateContent::File(plan) => Some(&plan.plan_id),
                                _ => None,
                            };
                            context_key = id.map(|id| format!("plan:{id}"));
                        }
                        acp::SessionUpdate::PlanRemoved(update) => {
                            context_key = Some(format!("plan:{}", update.plan_id));
                            removed = true;
                        }
                        acp::SessionUpdate::CompactionUpdate(update) => {
                            floor = self
                                .identity(format!("compaction:{}", update.compaction_id), cursor);
                        }
                        acp::SessionUpdate::CompactionSummaryChunk(update) => {
                            floor = self
                                .identity(format!("compaction:{}", update.compaction_id), cursor);
                        }
                        _ => {}
                    }
                }
            }
            EventKind::TurnStarted { .. } => {
                self.legacy = None;
                self.turn_start = Some(cursor);
                self.interruption = None;
                context_key = Some("turn".into());
            }
            EventKind::TurnFinished { .. } => {
                self.legacy = None;
                context_key = Some("turn".into());
                removed = true;
            }
            EventKind::TurnInterrupted { reason, .. } => {
                self.legacy = None;
                context_key = Some("turn".into());
                removed = true;
                self.interruption = match reason {
                    super::InterruptionReason::BackendFailure { .. }
                    | super::InterruptionReason::DispatchFailure { .. } => Some(cursor),
                    _ => None,
                };
            }
            EventKind::Failed { .. } => {
                self.legacy = None;
                context_key = Some("turn".into());
                removed = true;
                // Conversation consumes this state to suppress a duplicate failure notice.
                floor = self.interruption.take().unwrap_or(cursor);
            }
            _ => {}
        }
        let context_start = context_key.as_ref().and_then(|key| {
            if removed {
                self.anchors.remove(key);
                None
            } else if key == "usage" || key == "turn" {
                Some(event.cursor)
            } else {
                let mut start = cursor;
                self.reference(key.clone(), cursor, &mut start);
                Some(Cursor::from_raw(start))
            }
        });
        Dependency {
            floor: Cursor::from_raw(floor),
            context_key,
            context_start,
        }
    }

    fn identity(&mut self, key: String, cursor: u64) -> u64 {
        let existed = self.anchors.contains_key(&key);
        let mut start = cursor;
        self.reference(key, cursor, &mut start);
        if !existed {
            self.legacy = None;
        }
        start
    }

    fn reference(&mut self, key: String, cursor: u64, floor: &mut u64) {
        let first = if self.incomplete { 0 } else { cursor };
        let anchor = self.anchors.entry(key).or_insert(Anchor {
            first,
            last: cursor,
        });
        anchor.last = cursor;
        *floor = (*floor).min(anchor.first);
    }
}

fn terminal_references(
    meta: Option<&acp::Meta>,
    content: &[acp::ToolCallContent],
    reference: &mut impl FnMut(String),
) {
    if let Some(meta) = meta {
        for key in ["terminal_info", "terminal_output", "terminal_exit"] {
            if let Some(id) = meta
                .get(key)
                .and_then(|v| v.get("terminal_id"))
                .and_then(|v| v.as_str())
            {
                reference(format!("terminal:{id}"));
            }
        }
    }
    for content in content {
        if let acp::ToolCallContent::Terminal(terminal) = content {
            reference(format!("terminal:{}", terminal.terminal_id));
        }
    }
}
