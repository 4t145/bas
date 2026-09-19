use basr::{acp, model::*};
use std::collections::HashMap;
mod terminal;

pub(super) enum Item {
    Message { message: Message, streaming: bool },
    Tool(Box<ToolCall>),
    Notice { id: u64, text: String },
}

impl Item {
    pub fn id(&self) -> String {
        match self {
            Self::Message { message, .. } => format!("message-{}", message.id),
            Self::Tool(tool) => format!("tool-{}", tool.tool_call_id),
            Self::Notice { id, .. } => format!("notice-{id}"),
        }
    }
}

pub(super) struct Conversation {
    pub session: SessionMeta,
    pub items: Vec<Item>,
    pub terminals: HashMap<String, terminal::Terminal>,
    pub cursor: u64,
    pub commands: Vec<acp::AvailableCommand>,
    pub plan: Option<acp::Plan>,
    pub usage: Option<acp::UsageUpdate>,
    pub completed_reply: Option<CompletedReply>,
    turn_start: usize,
}

#[derive(Clone, Copy)]
pub(super) struct CompletedReply {
    pub index: usize,
    pub cursor: u64,
}

impl Conversation {
    pub fn new(session: SessionMeta) -> Self {
        Self {
            commands: session
                .available_commands
                .as_ref()
                .map(|update| update.available_commands.clone())
                .unwrap_or_default(),
            session,
            items: Vec::new(),
            terminals: HashMap::new(),
            cursor: 0,
            plan: None,
            usage: None,
            completed_reply: None,
            turn_start: 0,
        }
    }

    /// Returns the first item whose layout changed, if any.
    pub fn apply(&mut self, event: Event) -> Option<usize> {
        if event.session != self.session.id || event.cursor.raw() <= self.cursor {
            return None;
        }
        self.cursor = event.cursor.raw();
        self.session.updated_at = event.at;
        match event.kind {
            EventKind::Acp(update) => {
                self.session.apply_acp(&update);
                match update {
                    acp::SessionUpdate::UserMessageChunk(chunk) => {
                        self.chunk(Role::User, chunk.content, event.at)
                    }
                    acp::SessionUpdate::AgentMessageChunk(chunk) => {
                        self.chunk(Role::Agent, chunk.content, event.at)
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
                    acp::SessionUpdate::AvailableCommandsUpdate(update) => {
                        self.commands = update.available_commands;
                        None
                    }
                    acp::SessionUpdate::Plan(plan) => {
                        self.plan = Some(plan);
                        None
                    }
                    acp::SessionUpdate::UsageUpdate(usage) => {
                        self.usage = Some(usage);
                        None
                    }
                    _ => None,
                }
            }
            EventKind::TurnStarted { .. } => {
                self.session.status = SessionStatus::Working;
                self.turn_start = self.items.len();
                self.completed_reply = None;
                self.seal()
            }
            EventKind::TurnFinished { result, .. } => {
                self.session.status = SessionStatus::Idle;
                let changed = self.seal();
                self.completed_reply = self.items[self.turn_start..]
                    .iter()
                    .rposition(|item| matches!(item, Item::Message { message, .. } if message.role == Role::Agent))
                    .map(|offset| CompletedReply {
                        index: self.turn_start + offset,
                        cursor: self.cursor,
                    });
                let notice = match result.map(|response| response.stop_reason) {
                    Ok(StopReason::EndTurn) => None,
                    Ok(StopReason::Cancelled) => self.notice("本轮已停止"),
                    Ok(StopReason::MaxTokens | StopReason::MaxTurnRequests) => {
                        self.notice("本轮达到输出或请求上限")
                    }
                    Ok(StopReason::Refusal) => self.notice("Agent 拒绝了本轮请求"),
                    Err(error) => self.notice(format!("本轮执行失败：{error}")),
                    _ => self.notice("本轮已结束"),
                };
                changed.or(notice)
            }
            EventKind::TitleChanged { title } => {
                self.session.title = Some(title);
                None
            }
            EventKind::StatusChanged { status } => {
                self.session.status = status;
                None
            }
            EventKind::Failed { message } => {
                self.session.status = SessionStatus::Failed;
                let changed = self.seal();
                let notice = self.notice(format!("Agent 已断开：{message}"));
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

#[cfg(test)]
mod tests;
