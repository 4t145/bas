use crate::model::AgentId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// How a backend is driven.
pub enum Kind {
    Embedded,
    Acp,
}
impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
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
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("unknown agent kind {value:?}; expected embedded or acp")]
pub struct ParseKindError {
    value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// An executable and its argument boundaries; JSON never invokes shell parsing.
pub struct Command {
    pub program: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}
impl Command {
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().collect(),
        }
    }
}
impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.program.display())?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("command line has no program")]
pub struct ParseCommandError;
impl std::str::FromStr for Command {
    type Err = ParseCommandError;
    /// Splits whitespace without interpreting shell quoting.
    /// # Errors
    /// Rejects a command without an executable.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parts = text.split_whitespace();
        Ok(Self::new(
            parts.next().ok_or(ParseCommandError)?,
            parts.map(str::to_owned),
        ))
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Saved launch configuration and optional display metadata for one backend.
pub struct AgentSpec {
    pub id: AgentId,
    pub kind: Kind,
    /// User-provided notes, displayed by clients but not passed to the process.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Required for ACP and forbidden for embedded agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
    /// Environment overrides applied to the child process.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Maximum wait for a running prompt after sending cancellation.
    #[serde(
        default = "AgentSpec::default_cancellation_grace",
        rename = "cancellation_grace_ms",
        with = "milliseconds"
    )]
    pub cancellation_grace: Duration,
}
mod milliseconds {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    /// # Errors
    /// Rejects millisecond counts larger than the catalog format can represent.
    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        let millis = u64::try_from(duration.as_millis()).map_err(serde::ser::Error::custom)?;
        serializer.serialize_u64(millis)
    }
    /// # Errors
    /// Rejects negative, fractional, or overflowing millisecond counts.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }
}
impl AgentSpec {
    pub const DEFAULT_ID: &'static str = "embedded";
    pub const fn default_cancellation_grace() -> Duration {
        Duration::from_secs(5)
    }
    #[must_use]
    pub fn embedded() -> Self {
        Self {
            id: AgentId::new(Self::DEFAULT_ID),
            kind: Kind::Embedded,
            description: String::new(),
            command: None,
            env: BTreeMap::new(),
            cancellation_grace: Self::default_cancellation_grace(),
        }
    }
    #[must_use]
    pub fn acp(id: impl Into<AgentId>, command: Command) -> Self {
        Self {
            id: id.into(),
            kind: Kind::Acp,
            command: Some(command),
            ..Self::embedded()
        }
    }
    #[must_use]
    pub fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }
}
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ParseSpecError {
    #[error("agent entry {entry:?} has no id before '='")]
    MissingId { entry: String },
    #[error("agent {id} has an unknown kind: {source}")]
    Kind {
        id: String,
        #[source]
        source: ParseKindError,
    },
    #[error("agent {id} is of kind {kind} but has no command line")]
    MissingCommand { id: String, kind: Kind },
    #[error("embedded agent {id} cannot have an external command")]
    UnexpectedCommand { id: String },
}
impl std::str::FromStr for AgentSpec {
    type Err = ParseSpecError;
    /// Parses an id, id=kind, or id=kind:command line.
    /// # Errors
    /// Rejects missing IDs, unknown kinds, and commands incompatible with the kind.
    fn from_str(entry: &str) -> Result<Self, Self::Err> {
        let entry = entry.trim();
        let (id, rest) = entry
            .split_once('=')
            .map_or((entry, None), |(id, rest)| (id.trim(), Some(rest.trim())));
        if id.is_empty() {
            return Err(ParseSpecError::MissingId {
                entry: entry.to_owned(),
            });
        }
        let Some(rest) = rest else {
            return Ok(Self {
                id: id.into(),
                ..Self::embedded()
            });
        };
        let (kind, command) = rest
            .split_once(':')
            .map_or((rest, None), |(kind, command)| {
                (kind.trim(), Some(command.trim()))
            });
        let kind = kind
            .parse::<Kind>()
            .map_err(|source| ParseSpecError::Kind {
                id: id.to_owned(),
                source,
            })?;
        let command = command
            .filter(|c| !c.is_empty())
            .map(str::parse)
            .transpose()
            .map_err(|_: ParseCommandError| ParseSpecError::MissingCommand {
                id: id.to_owned(),
                kind,
            })?;
        if kind == Kind::Acp && command.is_none() {
            return Err(ParseSpecError::MissingCommand {
                id: id.to_owned(),
                kind,
            });
        }
        if kind == Kind::Embedded && command.is_some() {
            return Err(ParseSpecError::UnexpectedCommand { id: id.to_owned() });
        }
        Ok(Self {
            id: id.into(),
            kind,
            command,
            ..Self::embedded()
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub String);

/// # Errors
/// Rejects empty catalogs, duplicate/empty IDs, invalid commands and zero grace.
pub fn validate_agents(agents: &[AgentSpec]) -> Result<(), ValidationError> {
    if agents.is_empty() {
        return Err(ValidationError("agents must not be empty".into()));
    }
    let mut ids = HashSet::new();
    for agent in agents {
        if agent.id.as_str().trim().is_empty() {
            return Err(ValidationError("agent id must not be empty".into()));
        }
        if !ids.insert(&agent.id) {
            return Err(ValidationError(format!("duplicate agent id {}", agent.id)));
        }
        match (agent.kind, &agent.command) {
            (Kind::Acp, None) => {
                return Err(ValidationError(format!(
                    "agent {} requires a command",
                    agent.id
                )));
            }
            (Kind::Acp, Some(command)) if command.program.as_os_str().is_empty() => {
                return Err(ValidationError(format!(
                    "agent {} has an empty program",
                    agent.id
                )));
            }
            (Kind::Embedded, Some(_)) => {
                return Err(ValidationError(format!(
                    "embedded agent {} cannot have a command",
                    agent.id
                )));
            }
            _ => {}
        }
        if agent.cancellation_grace.is_zero() {
            return Err(ValidationError(format!(
                "agent {} has zero cancellation grace",
                agent.id
            )));
        }
    }
    Ok(())
}
