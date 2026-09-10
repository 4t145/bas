use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::AgentId;

/// How a backend is driven.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An agent linked into this daemon.
    Embedded,
    /// An external process spoken to over the Agent Client Protocol.
    Acp,
}

impl std::fmt::Display for Kind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Embedded => "embedded",
            Self::Acp => "acp",
        })
    }
}

impl std::str::FromStr for Kind {
    type Err = ParseKindError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "embedded" | "builtin" => Ok(Self::Embedded),
            "acp" => Ok(Self::Acp),
            _ => Err(ParseKindError {
                value: value.to_owned(),
            }),
        }
    }
}

/// Error returned when an agent kind name is unknown.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("unknown agent kind {value:?}; expected embedded or acp")]
pub struct ParseKindError {
    value: String,
}

/// The command line of an external backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub program: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl Command {
    /// Builds a command from a program and its fixed arguments.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().collect(),
        }
    }
}

impl std::fmt::Display for Command {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.program.display())?;
        for arg in &self.args {
            write!(formatter, " {arg}")?;
        }
        Ok(())
    }
}

/// Everything the daemon needs to launch one backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub id: AgentId,
    pub kind: Kind,
    /// Required for [`Kind::Acp`], ignored for [`Kind::Embedded`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
    /// Environment overrides applied to the child process.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl AgentSpec {
    /// The backend used when a session does not name one.
    pub const DEFAULT_ID: &'static str = "embedded";

    /// Builds a spec for the in-process agent.
    #[must_use]
    pub fn embedded() -> Self {
        Self {
            id: AgentId::new(Self::DEFAULT_ID),
            kind: Kind::Embedded,
            command: None,
            env: BTreeMap::new(),
        }
    }

    /// Builds a spec for an external ACP agent.
    #[must_use]
    pub fn acp(id: impl Into<AgentId>, command: Command) -> Self {
        Self {
            id: id.into(),
            kind: Kind::Acp,
            command: Some(command),
            env: BTreeMap::new(),
        }
    }

    /// Returns the command line, if this backend needs one.
    #[must_use]
    pub fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }
}
