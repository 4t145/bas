use super::{Cursor, SessionId, Timestamp, TurnId};
use crate::acp;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[cfg(test)]
mod tests;

/// A completed ACP prompt, or a daemon/backend failure.
pub type TurnResult = Result<acp::PromptResponse, String>;

/// ACP updates retain their complete schema; other variants describe daemon lifecycle.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "ACP updates dominate the stream; inline payloads avoid an allocation per chunk"
)]
pub enum EventKind {
    Acp(#[serde(serialize_with = "serialize_acp")] acp::SessionUpdate),
    TurnStarted { turn: TurnId },
    TurnFinished { turn: TurnId, result: TurnResult },
    TitleChanged { title: String },
    ArchiveChanged { archived: bool },
    StatusChanged { status: super::SessionStatus },
    Failed { message: String },
}

fn serialize_acp<S: Serializer>(
    update: &acp::SessionUpdate,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Payload<'a> {
        update: &'a acp::SessionUpdate,
    }
    Payload { update }.serialize(serializer)
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Earlier records flattened ACP into the lifecycle envelope. ToolCall's
        // own `kind` then duplicated the discriminator; retain that native key.
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("sessionUpdate").is_some() {
            return serde_json::from_value(value)
                .map(Self::Acp)
                .map_err(serde::de::Error::custom);
        }
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        #[expect(
            clippy::large_enum_variant,
            reason = "temporary owned representation of the same inline event payload"
        )]
        enum Stored {
            Acp { update: acp::SessionUpdate },
            TurnStarted { turn: TurnId },
            TurnFinished { turn: TurnId, result: TurnResult },
            TitleChanged { title: String },
            ArchiveChanged { archived: bool },
            StatusChanged { status: super::SessionStatus },
            Failed { message: String },
        }
        let stored: Stored = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(match stored {
            Stored::Acp { update } => Self::Acp(update),
            Stored::TurnStarted { turn } => Self::TurnStarted { turn },
            Stored::TurnFinished { turn, result } => Self::TurnFinished { turn, result },
            Stored::TitleChanged { title } => Self::TitleChanged { title },
            Stored::ArchiveChanged { archived } => Self::ArchiveChanged { archived },
            Stored::StatusChanged { status } => Self::StatusChanged { status },
            Stored::Failed { message } => Self::Failed { message },
        })
    }
}

/// Ordered, replayable session event. ACP chunks use the same cursor as lifecycle events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub session: SessionId,
    pub cursor: Cursor,
    pub at: Timestamp,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    /// Live notices have no replay cursor and never enter session history.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(&self.kind, EventKind::Acp(acp::SessionUpdate::Notice(_)))
    }

    #[must_use]
    pub fn new(session: SessionId, cursor: Cursor, kind: EventKind) -> Self {
        Self {
            session,
            cursor,
            at: Timestamp::now(),
            kind,
        }
    }
}
