//! Conversions between the domain model and the protobuf wire types.
//!
//! The server converts `model` values into protobuf; clients convert protobuf
//! back so that TUI and GUI code shares the daemon's types instead of the
//! generated ones.

use std::{path::PathBuf, str::FromStr};

use crate::model;

use super::pb;

/// Failure converting a protobuf value into the domain model.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown {field} {value:?}")]
    Unknown { field: &'static str, value: String },
    #[error("the event carries no kind")]
    MissingEventKind,
    #[error("the content block carries no block")]
    MissingContentBlock,
    #[error("invalid identifier: {0}")]
    Id(#[from] model::ParseIdError),
}

fn role_name(role: model::Role) -> &'static str {
    match role {
        model::Role::User => "user",
        model::Role::Agent => "agent",
    }
}

fn parse_role(value: &str) -> Result<model::Role, Error> {
    match value {
        "user" => Ok(model::Role::User),
        "agent" => Ok(model::Role::Agent),
        other => Err(Error::Unknown {
            field: "role",
            value: other.to_owned(),
        }),
    }
}

fn tool_kind_name(kind: model::ToolKind) -> &'static str {
    match kind {
        model::ToolKind::Read => "read",
        model::ToolKind::Edit => "edit",
        model::ToolKind::Delete => "delete",
        model::ToolKind::Move => "move",
        model::ToolKind::Search => "search",
        model::ToolKind::Execute => "execute",
        model::ToolKind::Fetch => "fetch",
        model::ToolKind::Think => "think",
        model::ToolKind::Other => "other",
    }
}

fn stop_name(stop: model::StopReason) -> &'static str {
    match stop {
        model::StopReason::EndTurn => "end_turn",
        model::StopReason::MaxTokens => "max_tokens",
        model::StopReason::Cancelled => "cancelled",
        model::StopReason::Refusal => "refusal",
        model::StopReason::Other => "other",
    }
}

fn parse_stop(value: &str) -> Result<model::StopReason, Error> {
    match value {
        "end_turn" => Ok(model::StopReason::EndTurn),
        "max_tokens" => Ok(model::StopReason::MaxTokens),
        "cancelled" => Ok(model::StopReason::Cancelled),
        "refusal" => Ok(model::StopReason::Refusal),
        "other" => Ok(model::StopReason::Other),
        other => Err(Error::Unknown {
            field: "stop reason",
            value: other.to_owned(),
        }),
    }
}

fn status_name(status: model::SessionStatus) -> &'static str {
    match status {
        model::SessionStatus::Starting => "starting",
        model::SessionStatus::Idle => "idle",
        model::SessionStatus::Working => "working",
        model::SessionStatus::Stopped => "stopped",
        model::SessionStatus::Failed => "failed",
    }
}

impl From<model::SessionStatus> for pb::SessionStatus {
    fn from(status: model::SessionStatus) -> Self {
        match status {
            model::SessionStatus::Starting => Self::Starting,
            model::SessionStatus::Idle => Self::Idle,
            model::SessionStatus::Working => Self::Working,
            model::SessionStatus::Stopped => Self::Stopped,
            model::SessionStatus::Failed => Self::Failed,
        }
    }
}

impl TryFrom<i32> for model::SessionStatus {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match pb::SessionStatus::try_from(value) {
            Ok(pb::SessionStatus::Starting) => Ok(Self::Starting),
            Ok(pb::SessionStatus::Idle) => Ok(Self::Idle),
            Ok(pb::SessionStatus::Working) => Ok(Self::Working),
            Ok(pb::SessionStatus::Stopped) => Ok(Self::Stopped),
            Ok(pb::SessionStatus::Failed) => Ok(Self::Failed),
            Ok(pb::SessionStatus::Unspecified) | Err(_) => Err(Error::Unknown {
                field: "session status",
                value: value.to_string(),
            }),
        }
    }
}

impl From<&model::Owner> for pb::Owner {
    fn from(owner: &model::Owner) -> Self {
        Self {
            user: owner.user.clone(),
            host: owner.host.clone(),
        }
    }
}

impl From<pb::Owner> for model::Owner {
    fn from(owner: pb::Owner) -> Self {
        Self {
            user: owner.user,
            host: owner.host,
        }
    }
}

impl From<&model::SessionMeta> for pb::Session {
    fn from(meta: &model::SessionMeta) -> Self {
        Self {
            id: meta.id.to_string(),
            agent: meta.agent.to_string(),
            title: meta.title.clone(),
            owner: Some((&meta.owner).into()),
            cwd: meta
                .cwd
                .as_ref()
                .map(|cwd| cwd.to_string_lossy().into_owned()),
            created_at: meta.created_at.millis(),
            updated_at: meta.updated_at.millis(),
            status: pb::SessionStatus::from(meta.status) as i32,
        }
    }
}

impl TryFrom<pb::Session> for model::SessionMeta {
    type Error = Error;

    fn try_from(session: pb::Session) -> Result<Self, Self::Error> {
        Ok(Self {
            id: model::SessionId::from_str(&session.id)?,
            agent: session.agent.into(),
            title: session.title,
            owner: session.owner.map_or_else(model::Owner::current, Into::into),
            cwd: session.cwd.map(PathBuf::from),
            created_at: model::Timestamp::from_millis(session.created_at),
            updated_at: model::Timestamp::from_millis(session.updated_at),
            status: session.status.try_into()?,
        })
    }
}

impl From<&model::ToolCall> for pb::ToolCall {
    fn from(call: &model::ToolCall) -> Self {
        Self {
            id: call.id.as_str().to_owned(),
            title: call.title.clone(),
            kind: tool_kind_name(call.kind).to_owned(),
            status: match call.status {
                model::ToolCallStatus::Pending => pb::ToolCallStatus::Pending as i32,
                model::ToolCallStatus::InProgress => pb::ToolCallStatus::InProgress as i32,
                model::ToolCallStatus::Completed => pb::ToolCallStatus::Completed as i32,
                model::ToolCallStatus::Failed => pb::ToolCallStatus::Failed as i32,
            },
            input_json: call.input.as_ref().map(serde_json::Value::to_string),
            output_json: call.output.as_ref().map(serde_json::Value::to_string),
            locations: call.locations.clone(),
        }
    }
}

impl TryFrom<pb::ToolCall> for model::ToolCall {
    type Error = Error;

    fn try_from(call: pb::ToolCall) -> Result<Self, Self::Error> {
        let status = match pb::ToolCallStatus::try_from(call.status) {
            Ok(pb::ToolCallStatus::InProgress) => model::ToolCallStatus::InProgress,
            Ok(pb::ToolCallStatus::Completed) => model::ToolCallStatus::Completed,
            Ok(pb::ToolCallStatus::Failed) => model::ToolCallStatus::Failed,
            Ok(pb::ToolCallStatus::Pending | pb::ToolCallStatus::Unspecified) | Err(_) => {
                model::ToolCallStatus::Pending
            }
        };

        Ok(Self {
            id: model::ToolCallId::new(call.id),
            title: call.title,
            kind: match call.kind.as_str() {
                "read" => model::ToolKind::Read,
                "edit" => model::ToolKind::Edit,
                "delete" => model::ToolKind::Delete,
                "move" => model::ToolKind::Move,
                "search" => model::ToolKind::Search,
                "execute" => model::ToolKind::Execute,
                "fetch" => model::ToolKind::Fetch,
                "think" => model::ToolKind::Think,
                _ => model::ToolKind::Other,
            },
            status,
            input: parse_json(call.input_json.as_deref()),
            output: parse_json(call.output_json.as_deref()),
            locations: call.locations,
        })
    }
}

impl From<&model::ContentBlock> for pb::ContentBlock {
    fn from(block: &model::ContentBlock) -> Self {
        use pb::content_block::Block;

        Self {
            block: Some(match block {
                model::ContentBlock::Text(text) => Block::Text(text.text.clone()),
                model::ContentBlock::Thinking(text) => Block::Thinking(text.text.clone()),
                model::ContentBlock::ToolCall(call) => Block::ToolCall(call.into()),
                model::ContentBlock::Resource(link) => Block::Resource(pb::ResourceLink {
                    uri: link.uri.clone(),
                    name: link.name.clone(),
                    mime: link.mime.clone(),
                }),
            }),
        }
    }
}

impl TryFrom<pb::ContentBlock> for model::ContentBlock {
    type Error = Error;

    fn try_from(block: pb::ContentBlock) -> Result<Self, Self::Error> {
        use pb::content_block::Block;

        match block.block.ok_or(Error::MissingContentBlock)? {
            Block::Text(text) => Ok(Self::Text(model::Text::new(text))),
            Block::Thinking(text) => Ok(Self::Thinking(model::Text::new(text))),
            Block::ToolCall(call) => Ok(Self::ToolCall(call.try_into()?)),
            Block::Resource(link) => Ok(Self::Resource(model::ResourceLink {
                uri: link.uri,
                name: link.name,
                mime: link.mime,
            })),
        }
    }
}

impl From<&model::Message> for pb::Message {
    fn from(message: &model::Message) -> Self {
        Self {
            id: message.id.to_string(),
            role: role_name(message.role).to_owned(),
            content: message.content.iter().map(Into::into).collect(),
            at: message.at.millis(),
        }
    }
}

impl TryFrom<pb::Message> for model::Message {
    type Error = Error;

    fn try_from(message: pb::Message) -> Result<Self, Self::Error> {
        Ok(Self {
            id: model::MessageId::from_str(&message.id)?,
            role: parse_role(&message.role)?,
            content: message
                .content
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            at: model::Timestamp::from_millis(message.at),
        })
    }
}

impl From<&model::Delta> for pb::Delta {
    fn from(delta: &model::Delta) -> Self {
        let (kind, text) = match &delta.block {
            model::DeltaBlock::Text { text } => ("text", text.clone()),
            model::DeltaBlock::Thinking { text } => ("thinking", text.clone()),
        };

        Self {
            message: delta.message.to_string(),
            role: role_name(delta.role).to_owned(),
            kind: kind.to_owned(),
            text,
        }
    }
}

impl TryFrom<pb::Delta> for model::Delta {
    type Error = Error;

    fn try_from(delta: pb::Delta) -> Result<Self, Self::Error> {
        let block = match delta.kind.as_str() {
            "text" => model::DeltaBlock::Text { text: delta.text },
            "thinking" => model::DeltaBlock::Thinking { text: delta.text },
            other => {
                return Err(Error::Unknown {
                    field: "delta kind",
                    value: other.to_owned(),
                });
            }
        };

        Ok(Self {
            message: model::MessageId::from_str(&delta.message)?,
            role: parse_role(&delta.role)?,
            block,
        })
    }
}

impl From<&model::Event> for pb::Event {
    fn from(event: &model::Event) -> Self {
        use pb::event::Kind;

        let kind = match &event.kind {
            model::EventKind::Delta(delta) => Kind::Delta(delta.into()),
            model::EventKind::Message(message) => Kind::Message(message.into()),
            model::EventKind::ToolCall(call) => Kind::ToolCall(call.into()),
            model::EventKind::TurnStarted { turn } => Kind::TurnStarted(pb::TurnStarted {
                turn: turn.to_string(),
            }),
            model::EventKind::TurnFinished { turn, stop } => Kind::TurnFinished(pb::TurnFinished {
                turn: turn.to_string(),
                stop: stop_name(*stop).to_owned(),
            }),
            model::EventKind::TitleChanged { title } => Kind::TitleChanged(pb::TitleChanged {
                title: title.clone(),
            }),
            model::EventKind::StatusChanged { status } => Kind::StatusChanged(pb::StatusChanged {
                status: pb::SessionStatus::from(*status) as i32,
            }),
        };

        Self {
            session: event.session.to_string(),
            cursor: event.cursor.raw(),
            at: event.at.millis(),
            kind: Some(kind),
        }
    }
}

impl TryFrom<pb::Event> for model::Event {
    type Error = Error;

    fn try_from(event: pb::Event) -> Result<Self, Self::Error> {
        use pb::event::Kind;

        let kind = match event.kind.ok_or(Error::MissingEventKind)? {
            Kind::Delta(delta) => model::EventKind::Delta(delta.try_into()?),
            Kind::Message(message) => model::EventKind::Message(message.try_into()?),
            Kind::ToolCall(call) => model::EventKind::ToolCall(call.try_into()?),
            Kind::TurnStarted(started) => model::EventKind::TurnStarted {
                turn: model::TurnId::from_str(&started.turn)?,
            },
            Kind::TurnFinished(finished) => model::EventKind::TurnFinished {
                turn: model::TurnId::from_str(&finished.turn)?,
                stop: parse_stop(&finished.stop)?,
            },
            Kind::TitleChanged(changed) => model::EventKind::TitleChanged {
                title: changed.title,
            },
            Kind::StatusChanged(changed) => model::EventKind::StatusChanged {
                status: changed.status.try_into()?,
            },
        };

        Ok(Self {
            session: model::SessionId::from_str(&event.session)?,
            cursor: model::Cursor::from_raw(event.cursor),
            at: model::Timestamp::from_millis(event.at),
            kind,
        })
    }
}

/// Renders a status for human-facing clients such as `basrctl`.
#[must_use]
pub fn status_label(status: model::SessionStatus) -> &'static str {
    status_name(status)
}

fn parse_json(value: Option<&str>) -> Option<serde_json::Value> {
    value.and_then(|value| serde_json::from_str(value).ok())
}
