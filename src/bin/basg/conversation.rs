use bas::{acp, acp_v2 as acp2, model::*};
use std::{collections::HashMap, ops::Range};
mod protocol;
mod response;
mod terminal;

pub(super) use protocol::{Compaction, ProtocolNotice, plan_id};

pub(super) enum Item {
    Message { message: Message, streaming: bool },
    Tool(Box<ToolCall>),
    Notice { id: u64, text: String },
    Compaction(Compaction),
}

impl Item {
    pub fn id(&self) -> String {
        match self {
            Self::Message { message, .. } => format!("message-{}", message.id),
            Self::Tool(tool) => format!("tool-{}", tool.tool_call_id),
            Self::Notice { id, .. } => format!("notice-{id}"),
            Self::Compaction(compaction) => format!("compaction-{}", compaction.id),
        }
    }

    fn is_activity(&self) -> bool {
        matches!(self, Self::Tool(_))
            || matches!(self, Self::Message { message, .. } if message.role == Role::Thought)
    }
}

pub(super) struct TimelineRow {
    pub items: Range<usize>,
    pub activity: bool,
}

pub(super) struct Conversation {
    pub session: SessionMeta,
    pub items: Vec<Item>,
    pub rows: Vec<TimelineRow>,
    pub terminals: HashMap<String, terminal::Terminal>,
    pub cursor: u64,
    pub plans: Vec<acp2::PlanUpdate>,
    pub usage: Option<acp2::UsageUpdate>,
    pub notices: Vec<ProtocolNotice>,
    pub foreground_state: Option<acp2::StateUpdate>,
    pub completed_reply: Option<ReplyPosition>,
    pub last_reply: Option<ReplyPosition>,
    turn_start: usize,
    next_notice: u64,
    interruption_message: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) struct ReplyPosition {
    pub index: usize,
    /// The last content chunk, matching SessionMeta::last_reply rather than TurnFinished.
    pub cursor: u64,
}

impl Conversation {
    pub fn merge_meta(&mut self, meta: &SessionMeta) {
        merge_meta(&mut self.session, meta);
    }

    pub fn commands(&self) -> &[acp::AvailableCommand] {
        self.session
            .available_commands
            .as_ref()
            .map_or(&[], |update| update.available_commands.as_slice())
    }

    pub fn new(session: SessionMeta) -> Self {
        Self {
            session,
            items: Vec::new(),
            rows: Vec::new(),
            terminals: HashMap::new(),
            cursor: 0,
            plans: Vec::new(),
            usage: None,
            notices: Vec::new(),
            foreground_state: None,
            completed_reply: None,
            last_reply: None,
            turn_start: 0,
            next_notice: 0,
            interruption_message: None,
        }
    }

    /// Returns the first item whose layout changed, if any.
    pub fn apply(&mut self, event: Event) -> Option<usize> {
        let changed = self.apply_event(event);
        self.append_rows();
        changed
    }

    pub fn row_for_item(&self, index: usize) -> Option<usize> {
        let row = self.rows.partition_point(|row| row.items.end <= index);
        self.rows
            .get(row)
            .filter(|row| row.items.contains(&index))
            .map(|_| row)
    }

    fn append_rows(&mut self) {
        let start = self.rows.last().map_or(0, |row| row.items.end);
        for index in start..self.items.len() {
            let activity = self.items[index].is_activity();
            if activity
                && let Some(row) = self.rows.last_mut()
                && row.activity
                && row.items.start >= self.turn_start
            {
                row.items.end = index + 1;
            } else {
                self.rows.push(TimelineRow {
                    items: index..index + 1,
                    activity,
                });
            }
        }
    }

    fn apply_event(&mut self, event: Event) -> Option<usize> {
        if event.session != self.session.id {
            return None;
        }
        if let EventKind::Acp(acp::SessionUpdate::Notice(notice)) = &event.kind {
            if event.cursor == Cursor::START {
                match serde_json::to_value(notice).and_then(serde_json::from_value) {
                    Ok(notice) => self.push_notice(notice),
                    Err(error) => tracing::warn!(%error, "could not normalize ACP notice"),
                }
            }
            return None;
        }
        if event.cursor.raw() <= self.cursor {
            return None;
        }
        self.cursor = event.cursor.raw();
        self.session.apply_event(&event);
        match event.kind {
            EventKind::Acp(update) => match update {
                acp::SessionUpdate::UserMessageChunk(chunk) => {
                    self.chunk(Role::User, chunk.content, event.at)
                }
                acp::SessionUpdate::AgentMessageChunk(chunk) => {
                    let changed = self.chunk(Role::Agent, chunk.content, event.at);
                    self.last_reply = Some(ReplyPosition {
                        index: self.items.len() - 1,
                        cursor: self.cursor,
                    });
                    changed
                }
                acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                    self.chunk(Role::Thought, chunk.content, event.at)
                }
                acp::SessionUpdate::ToolCall(tool) => {
                    terminal::apply(&mut self.terminals, tool.meta.as_ref());
                    Some(self.upsert_tool(tool))
                }
                acp::SessionUpdate::ToolCallUpdate(update) => {
                    terminal::apply(&mut self.terminals, update.meta.as_ref());
                    let index = self.items.iter().position(|item| matches!(item, Item::Tool(tool) if tool.tool_call_id == update.tool_call_id));
                    if let Some(index) = index {
                        if let Item::Tool(tool) = &mut self.items[index] {
                            tool.update(update.fields);
                        }
                        Some(index)
                    } else {
                        let mut tool = ToolCall::new(update.tool_call_id, "");
                        tool.update(update.fields);
                        Some(self.upsert_tool(tool))
                    }
                }
                acp::SessionUpdate::Plan(plan) => {
                    self.apply_legacy_plan(&plan);
                    None
                }
                update @ (acp::SessionUpdate::PlanUpdate(_)
                | acp::SessionUpdate::PlanRemoved(_)
                | acp::SessionUpdate::UsageUpdate(_)
                | acp::SessionUpdate::CompactionUpdate(_)
                | acp::SessionUpdate::CompactionSummaryChunk(_)) => self.apply_v1_protocol(update),
                _ => None,
            },
            EventKind::TurnStarted { .. } => {
                self.interruption_message = None;
                self.foreground_state =
                    Some(acp2::StateUpdate::Running(acp2::RunningStateUpdate::new()));
                self.turn_start = self.items.len();
                self.completed_reply = None;
                self.seal()
            }
            EventKind::TurnFinished { result, .. } => {
                self.foreground_state = Some(acp2::StateUpdate::Idle(acp2::IdleStateUpdate::new()));
                let changed = self.seal();
                self.completed_reply = self
                    .last_reply
                    .filter(|reply| result.is_ok() && reply.index >= self.turn_start);
                let notice = match result.map(|response| response.stop_reason) {
                    Ok(StopReason::EndTurn) => None,
                    Ok(StopReason::Cancelled) => self.notice(crate::i18n::tr!("turn-stopped")),
                    Ok(StopReason::MaxTokens | StopReason::MaxTurnRequests) => {
                        self.notice(crate::i18n::tr!("turn-limit"))
                    }
                    Ok(StopReason::Refusal) => self.notice(crate::i18n::tr!("turn-refused")),
                    Err(error) => {
                        self.notice(crate::i18n::tr!("turn-failed", error = error.message))
                    }
                    _ => self.notice(crate::i18n::tr!("turn-ended")),
                };
                changed.or(notice)
            }
            EventKind::TurnInterrupted { reason, .. } => {
                self.foreground_state = Some(acp2::StateUpdate::Idle(acp2::IdleStateUpdate::new()));
                let changed = self.seal();
                self.completed_reply = None;
                self.interruption_message = match &reason {
                    InterruptionReason::BackendFailure { message }
                    | InterruptionReason::DispatchFailure { message } => Some(message.clone()),
                    _ => None,
                };
                let notice = self.notice(interruption_notice(reason));
                changed.or(notice)
            }
            EventKind::TitleChanged { .. }
            | EventKind::ArchiveChanged { .. }
            | EventKind::StatusChanged { .. } => None,
            EventKind::Failed { message } => {
                self.foreground_state = Some(acp2::StateUpdate::Idle(acp2::IdleStateUpdate::new()));
                let changed = self.seal();
                if self.interruption_message.take().as_ref() == Some(&message) {
                    return changed;
                }
                let notice = self.notice(crate::i18n::tr!(
                    "agent-disconnected",
                    message = (message).to_string()
                ));
                changed.or(notice)
            }
        }
    }

    fn chunk(&mut self, role: Role, content: ContentBlock, at: Timestamp) -> Option<usize> {
        if let Some(Item::Message {
            message,
            streaming: true,
        }) = self.items.last_mut()
            && message.role == role
        {
            match (message.content.last_mut(), &content) {
                (Some(ContentBlock::Text(old)), ContentBlock::Text(new))
                    if old.annotations == new.annotations && old.meta == new.meta =>
                {
                    old.text.push_str(&new.text)
                }
                _ => message.content.push(content),
            }
            return Some(self.items.len() - 1);
        }
        let changed = self.seal();
        let mut message = Message::new(role, vec![content]);
        message.at = at;
        self.items.push(Item::Message {
            message,
            streaming: true,
        });
        changed.or(Some(self.items.len() - 1))
    }

    fn seal(&mut self) -> Option<usize> {
        let mut first = None;
        for (index, item) in self.items.iter_mut().enumerate() {
            if let Item::Message { streaming, .. } = item
                && *streaming
            {
                *streaming = false;
                first.get_or_insert(index);
            }
        }
        first
    }

    fn upsert_tool(&mut self, tool: ToolCall) -> usize {
        if let Some(index) = self.items.iter().position(
            |item| matches!(item, Item::Tool(old) if old.tool_call_id == tool.tool_call_id),
        ) {
            self.items[index] = Item::Tool(Box::new(tool));
            index
        } else {
            let changed = self.seal();
            self.items.push(Item::Tool(Box::new(tool)));
            changed.unwrap_or(self.items.len() - 1)
        }
    }

    fn notice(&mut self, text: impl Into<String>) -> Option<usize> {
        self.items.push(Item::Notice {
            id: self.cursor,
            text: text.into(),
        });
        Some(self.items.len() - 1)
    }
}

fn interruption_notice(reason: InterruptionReason) -> String {
    match reason {
        InterruptionReason::BackendFailure { message } => {
            crate::i18n::tr!("turn-interrupted-backend", message = message)
        }
        InterruptionReason::DispatchFailure { message } => {
            crate::i18n::tr!("turn-interrupted-dispatch", message = message)
        }
        InterruptionReason::SessionStopped => crate::i18n::tr!("turn-interrupted-stopped").into(),
        InterruptionReason::DaemonRestarted => {
            crate::i18n::tr!("turn-interrupted-restarted").into()
        }
        InterruptionReason::CancelledBeforeDispatch => {
            crate::i18n::tr!("turn-interrupted-cancelled").into()
        }
    }
}

pub(super) fn merge_meta(current: &mut SessionMeta, incoming: &SessionMeta) -> bool {
    if current.id != incoming.id || incoming.revision <= current.revision {
        return false;
    }
    *current = incoming.clone();
    true
}

#[cfg(test)]
mod tests;
