use std::{env, net::SocketAddr, str::FromStr};

use anyhow::{Context, Result};

use crate::{agent::AgentSpec, session::Limits};

use super::parse_agents;

/// Address the gRPC service listens on.
pub const LISTEN_ENV: &str = "BASRD_LISTEN";
/// Comma-free, semicolon-separated agent catalog. See [`parse_agents`].
pub const AGENTS_ENV: &str = "BASRD_AGENTS";
/// Desktop notification policy: `auto`, `always`, or `never`.
pub const NOTIFY_ENV: &str = "BASRD_NOTIFY";
/// Replayable events kept per session.
pub const HISTORY_ENV: &str = "BASRD_HISTORY";

/// Loopback by default: the service is meant for local clients and a session
/// can read and write files on the user's behalf.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7337";
/// Replay window of a session, matching [`Limits`].
pub const DEFAULT_HISTORY: usize = 4096;

/// When the daemon sends desktop notifications.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Notify {
    /// Notify when a desktop session is reachable.
    #[default]
    Auto,
    /// Always try to notify, even without a session bus.
    Always,
    /// Never notify.
    Never,
}

impl std::fmt::Display for Notify {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        })
    }
}

impl FromStr for Notify {
    type Err = ParseNotifyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "always" | "on" => Ok(Self::Always),
            "never" | "off" => Ok(Self::Never),
            _ => Err(ParseNotifyError {
                value: value.to_owned(),
            }),
        }
    }
}

/// Error returned when a notification policy name is unknown.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("unknown notification policy {value:?}; expected auto, always, or never")]
pub struct ParseNotifyError {
    value: String,
}

/// Daemon configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub listen: SocketAddr,
    pub notify: Notify,
    pub agents: Vec<AgentSpec>,
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: DEFAULT_LISTEN
                .parse()
                .expect("the default listen address is a valid socket address"),
            notify: Notify::default(),
            agents: vec![AgentSpec::embedded()],
            limits: Limits {
                history_capacity: DEFAULT_HISTORY,
            },
        }
    }
}

impl Config {
    /// Reads the environment, keeping the default for every variable that is
    /// unset.
    ///
    /// # Errors
    ///
    /// Returns an error when a variable is present but invalid.
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();

        if let Some(value) = read_env(LISTEN_ENV)? {
            config.listen = value
                .parse()
                .with_context(|| format!("invalid {LISTEN_ENV}"))?;
        }

        if let Some(value) = read_env(NOTIFY_ENV)? {
            config.notify = value
                .parse()
                .with_context(|| format!("invalid {NOTIFY_ENV}"))?;
        }

        if let Some(value) = read_env(AGENTS_ENV)? {
            config.agents =
                parse_agents(&value).with_context(|| format!("invalid {AGENTS_ENV}"))?;
        }

        if let Some(value) = read_env(HISTORY_ENV)? {
            config.limits.history_capacity = value
                .parse()
                .with_context(|| format!("invalid {HISTORY_ENV}"))?;
        }

        Ok(config)
    }
}

fn read_env(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("{name} is not valid UTF-8")),
    }
}
