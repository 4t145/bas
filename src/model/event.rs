use super::{Cursor, SessionId, Timestamp, TurnId};
use crate::acp;
use serde::{Deserialize, Serialize};

/// A completed ACP prompt, or a daemon/backend failure.
pub type TurnResult = Result<acp::PromptResponse, String>;

/// ACP updates retain their complete schema; other variants describe daemon lifecycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "ACP updates dominate the stream; inline payloads avoid an allocation per chunk"
)]
pub enum EventKind {
    Acp(acp::SessionUpdate),
    TurnStarted { turn: TurnId },
    TurnFinished { turn: TurnId, result: TurnResult },
    TitleChanged { title: String },
    StatusChanged { status: super::SessionStatus },
    Failed { message: String },
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
