use serde::{Deserialize, Serialize};

use super::{
    ContentBlock, Cursor, Message, MessageId, Role, SessionId, Timestamp, ToolCall, TurnId,
};

/// Why a turn stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The agent finished the turn on its own.
    EndTurn,
    /// The agent hit its output limit.
    MaxTokens,
    /// A client asked to cancel.
    Cancelled,
    /// The agent declined to continue.
    Refusal,
    /// The backend reported a reason this daemon does not model.
    Other,
}

/// A chunk of a message that is still being produced.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Delta {
    pub message: MessageId,
    pub role: Role,
    pub block: DeltaBlock,
}

/// The growing part of a streaming message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeltaBlock {
    Text { text: String },
    Thinking { text: String },
}

impl DeltaBlock {
    /// Appends this chunk to an accumulating content block.
    pub fn apply(self, blocks: &mut Vec<ContentBlock>) {
        let (target, text) = match self {
            Self::Text { text } => (false, text),
            Self::Thinking { text } => (true, text),
        };

        match (blocks.last_mut(), target) {
            (Some(ContentBlock::Text(existing)), false) => existing.text.push_str(&text),
            (Some(ContentBlock::Thinking(existing)), true) => existing.text.push_str(&text),
            _ if target => blocks.push(ContentBlock::Thinking(super::Text::new(text))),
            _ => blocks.push(ContentBlock::text(text)),
        }
    }
}

/// What happened, without the session-scoped envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// A chunk of a message that is still streaming. Deltas are live-only: they
    /// are never written to the replay history.
    Delta(Delta),
    /// A completed message.
    Message(Message),
    /// A tool call changed state.
    ToolCall(ToolCall),
    /// A turn began.
    TurnStarted { turn: TurnId },
    /// A turn finished.
    TurnFinished { turn: TurnId, stop: StopReason },
    /// The agent or a client changed the session title.
    TitleChanged { title: String },
    /// The session left its previous lifecycle state.
    StatusChanged { status: super::SessionStatus },
}

/// A normalized, session-scoped event.
///
/// The envelope carries everything a client needs to place the event in a
/// session's timeline, so frontends never inspect backend-specific payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub session: SessionId,
    pub cursor: Cursor,
    pub at: Timestamp,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    /// Wraps an event kind with its session, cursor, and timestamp.
    #[must_use]
    pub fn new(session: SessionId, cursor: Cursor, kind: EventKind) -> Self {
        Self {
            session,
            cursor,
            at: Timestamp::now(),
            kind,
        }
    }

    /// Returns whether this event belongs in the replay history.
    ///
    /// Streaming deltas are reconstructed from the committed message, so
    /// replaying them would duplicate content.
    #[must_use]
    pub fn is_replayable(&self) -> bool {
        !matches!(self.kind, EventKind::Delta(_))
    }

    /// Returns the message carried by this event, if any.
    #[must_use]
    pub fn message(&self) -> Option<&Message> {
        match &self.kind {
            EventKind::Message(message) => Some(message),
            _ => None,
        }
    }
}
