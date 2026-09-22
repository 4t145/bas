use std::{
    io,
    path::{Path, PathBuf},
};

use crate::agent::{AgentSpec, Kind, ParseSpecError};

pub const AGENTS_FILE_ENV: &str = "BASRD_AGENTS_FILE";

#[derive(Debug, thiserror::Error)]
pub enum AgentsFileError {
    #[error("cannot read agent catalog {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid agent catalog {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("agent catalog {0} must contain at least one agent")]
    Empty(PathBuf),
    #[error("invalid agent {id:?} in {path}: {reason}")]
    Invalid {
        path: PathBuf,
        id: String,
        reason: &'static str,
    },
}

/// Reads the native agent specifications, preserving argument and environment boundaries.
///
/// # Errors
/// Returns an error for unreadable files, malformed JSON, or invalid backend commands.
pub fn read_agents(path: &Path) -> Result<Vec<AgentSpec>, AgentsFileError> {
    let source = std::fs::read(path).map_err(|source| AgentsFileError::Read {
        path: path.into(),
        source,
    })?;
    let agents: Vec<AgentSpec> =
        serde_json::from_slice(&source).map_err(|source| AgentsFileError::Json {
            path: path.into(),
            source,
        })?;
    if agents.is_empty() {
        return Err(AgentsFileError::Empty(path.into()));
    }
    for agent in &agents {
        let reason = if agent.id.as_str().trim().is_empty() {
            Some("id must not be empty")
        } else if agent.cancellation_grace.is_zero() {
            Some("cancellation_grace_ms must be greater than zero")
        } else if agent.kind == Kind::Embedded && agent.command.is_some() {
            Some("embedded agent cannot have an external command")
        } else if agent.kind == Kind::Acp
            && agent
                .command
                .as_ref()
                .is_none_or(|command| command.program.as_os_str().is_empty())
        {
            Some("ACP agent requires an executable program")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(AgentsFileError::Invalid {
                path: path.into(),
                id: agent.id.to_string(),
                reason,
            });
        }
    }
    Ok(agents)
}

/// Parses the `BASRD_AGENTS` environment variable.
///
/// Entries are separated by `;`. An entry is `id`, `id=kind`, or
/// `id=kind:command line`, where `kind` is `embedded` or `acp` and the command
/// line is split on whitespace (no quoting).
///
/// # Errors
///
/// Returns [`ParseSpecError`] when an entry has no id, an unknown kind, or an
/// external kind without a command line.
pub fn parse_agents(value: &str) -> Result<Vec<AgentSpec>, ParseSpecError> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::parse)
        .collect()
}
