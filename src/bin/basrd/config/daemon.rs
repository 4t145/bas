use std::{net::SocketAddr, str::FromStr};

use basr::service::DEFAULT_ADDRESS;

use crate::{
    agent::{AgentSpec, PermissionPolicy},
    session::Limits,
};

/// Address the gRPC service listens on.
pub const LISTEN_ENV: &str = "BASRD_LISTEN";
/// Comma-free, semicolon-separated agent catalog. See [`super::parse_agents`].
pub const AGENTS_ENV: &str = "BASRD_AGENTS";
/// Desktop notification policy: `auto`, `always`, or `never`.
pub const NOTIFY_ENV: &str = "BASRD_NOTIFY";
/// Replayable events kept per session.
pub const HISTORY_ENV: &str = "BASRD_HISTORY";

pub const BUFFER_ENV: &str = "BASRD_BUFFER";
pub const STARTUP_TIMEOUT_ENV: &str = "BASRD_STARTUP_TIMEOUT";
pub const PERMISSIONS_ENV: &str = "BASRD_PERMISSIONS";

/// When the daemon sends desktop notifications.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Notify {
    /// Notify when a desktop session is reachable.
    #[default]
    Auto,
    /// Require an available desktop notification service at startup.
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
    pub permissions: PermissionPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: DEFAULT_ADDRESS
                .parse()
                .expect("the default listen address is a valid socket address"),
            notify: Notify::default(),
            agents: vec![AgentSpec::embedded()],
            limits: Limits::default(),
            permissions: PermissionPolicy::default(),
        }
    }
}
