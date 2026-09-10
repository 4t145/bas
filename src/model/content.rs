use serde::{Deserialize, Serialize};

/// A piece of agent or user content, normalized across agent backends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Visible text, either from the user or from the agent.
    Text(Text),
    /// Reasoning the agent chose to expose.
    Thinking(Text),
    /// A tool invocation together with its latest state.
    ToolCall(ToolCall),
    /// A reference to a file, URL, or other resource.
    Resource(ResourceLink),
}

impl ContentBlock {
    /// Builds a plain text block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(Text::new(text))
    }
}

/// A run of natural language.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub text: String,
}

impl Text {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

/// Identifier of a tool invocation, unique within a session.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCallId(String);

impl ToolCallId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a tool call is doing, used by frontends to pick an icon or verb.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Fetch,
    Think,
    #[default]
    Other,
}

/// Lifecycle of a tool call.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// A tool invocation and its latest known state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolCallStatus,
    /// Arguments as supplied by the agent, when the backend exposes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// Result payload, when the backend exposes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    /// Files the call touched, for frontends that follow the agent around.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<String>,
}

impl ToolCall {
    /// Creates a pending tool call.
    #[must_use]
    pub fn new(id: ToolCallId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            kind: ToolKind::Other,
            status: ToolCallStatus::Pending,
            input: None,
            output: None,
            locations: Vec::new(),
        }
    }
}

/// A reference to a resource the agent read or produced.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceLink {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}
