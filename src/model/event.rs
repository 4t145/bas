use super::{Cursor, SessionId, Timestamp, TurnId};
use crate::acp;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[cfg(test)]
mod tests;

/// The agent's native answer to an ACP prompt, never a daemon or storage failure.
pub type PromptResult = Result<acp::PromptResponse, acp::Error>;

/// Why BASR stopped tracking a prompt without receiving an ACP result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterruptionReason {
    BackendFailure { message: String },
    DispatchFailure { message: String },
    SessionStopped,
    DaemonRestarted,
    CancelledBeforeDispatch,
}

/// ACP updates retain their complete schema; other variants describe daemon lifecycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "ACP updates dominate the stream; inline payloads avoid an allocation per chunk"
)]
pub enum EventKind {
    Acp(
        #[serde(serialize_with = "serialize_acp", deserialize_with = "deserialize_acp")]
        acp::SessionUpdate,
    ),
    TurnStarted {
        turn: TurnId,
    },
    TurnFinished {
        turn: TurnId,
        result: PromptResult,
    },
    TurnInterrupted {
        turn: TurnId,
        reason: InterruptionReason,
    },
    TitleChanged {
        title: String,
    },
    ArchiveChanged {
        archived: bool,
    },
    StatusChanged {
        status: super::SessionStatus,
    },
    Failed {
        message: String,
    },
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

fn deserialize_acp<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<acp::SessionUpdate, D::Error> {
    #[derive(Deserialize)]
    struct Payload {
        update: acp::SessionUpdate,
    }
    Ok(Payload::deserialize(deserializer)?.update)
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
