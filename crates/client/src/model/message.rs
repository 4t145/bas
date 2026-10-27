use serde::{Deserialize, Serialize};

use super::{ContentBlock, MessageId, Timestamp};

/// Who produced a message.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Agent,
    Thought,
}

/// A client presentation message assembled from ACP content chunks.
/// This type is never sent over RPC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub role: Role,
    pub content: Vec<ContentBlock>,
    pub at: Timestamp,
}

impl Message {
    /// Creates a message with a fresh identifier.
    #[must_use]
    pub fn new(role: Role, content: Vec<ContentBlock>) -> Self {
        Self::with_id(MessageId::new(), role, content)
    }

    /// Creates a message with a caller-chosen identifier, for backends that
    /// stream a message before they can commit it.
    #[must_use]
    pub fn with_id(id: MessageId, role: Role, content: Vec<ContentBlock>) -> Self {
        Self {
            id,
            role,
            content,
            at: Timestamp::now(),
        }
    }

    /// Creates a single-block text message with a fresh identifier.
    #[must_use]
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self::new(role, vec![ContentBlock::from(text.into())])
    }

    /// Concatenates every text block, ignoring non-text content.
    #[must_use]
    pub fn plain_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}
