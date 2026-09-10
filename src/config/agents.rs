use crate::agent::{AgentSpec, Command, Kind, ParseKindError};

/// Error returned when `BASRD_AGENTS` is malformed.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ParseAgentsError {
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
}

/// Parses the `BASRD_AGENTS` environment variable.
///
/// Entries are separated by `;`. An entry is `id`, `id=kind`, or
/// `id=kind:command line`, where `kind` is `embedded` or `acp` and the command
/// line is split on whitespace (no quoting).
///
/// # Errors
///
/// Returns [`ParseAgentsError`] when an entry has no id, an unknown kind, or an
/// external kind without a command line.
pub fn parse_agents(value: &str) -> Result<Vec<AgentSpec>, ParseAgentsError> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_agent)
        .collect()
}

fn parse_agent(entry: &str) -> Result<AgentSpec, ParseAgentsError> {
    let (id, rest) = entry
        .split_once('=')
        .map_or((entry, None), |(id, rest)| (id.trim(), Some(rest.trim())));

    if id.is_empty() {
        return Err(ParseAgentsError::MissingId {
            entry: entry.to_owned(),
        });
    }

    let Some(rest) = rest else {
        return Ok(AgentSpec {
            id: id.into(),
            ..AgentSpec::embedded()
        });
    };

    let (kind, command) = rest
        .split_once(':')
        .map_or((rest, None), |(kind, command)| {
            (kind.trim(), Some(command.trim()))
        });

    let kind = kind
        .parse::<Kind>()
        .map_err(|source| ParseAgentsError::Kind {
            id: id.to_owned(),
            source,
        })?;

    let command = command
        .filter(|command| !command.is_empty())
        .map(command_line);

    if kind == Kind::Acp && command.is_none() {
        return Err(ParseAgentsError::MissingCommand {
            id: id.to_owned(),
            kind,
        });
    }

    Ok(AgentSpec {
        id: id.into(),
        kind,
        command,
        env: Default::default(),
    })
}

fn command_line(text: &str) -> Command {
    let mut parts = text.split_whitespace();
    let program = parts.next().unwrap_or_default();

    Command::new(program, parts.map(str::to_owned))
}
