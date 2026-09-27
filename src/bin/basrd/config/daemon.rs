use std::{net::SocketAddr, path::PathBuf, str::FromStr};

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
/// Storage backend: `memory` or `localdb`.
pub const STORAGE_ENV: &str = "BASRD_STORAGE";
/// SQLite file path when using `localdb`.
pub const LOCALDB_PATH_ENV: &str = "BASRD_LOCALDB_PATH";

pub const BUFFER_ENV: &str = "BASRD_BUFFER";
pub const STARTUP_TIMEOUT_ENV: &str = "BASRD_STARTUP_TIMEOUT";
pub const PERMISSIONS_ENV: &str = "BASRD_PERMISSIONS";

/// Storage for session metadata and replayable events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum Storage {
    /// Keep sessions only until the daemon stops.
    Memory,
    /// Persist sessions to a local SQLite database.
    #[default]
    Localdb,
}

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
    pub catalog_source: Option<super::CatalogSource>,
    pub limits: Limits,
    pub permissions: PermissionPolicy,
    pub storage: Storage,
    pub localdb_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: DEFAULT_ADDRESS
                .parse()
                .expect("the default listen address is a valid socket address"),
            notify: Notify::default(),
            agents: vec![AgentSpec::embedded()],
            catalog_source: None,
            limits: Limits::default(),
            permissions: PermissionPolicy::default(),
            storage: Storage::default(),
            localdb_path: None,
        }
    }
}

impl Config {
    /// Resolves the SQLite file only when local persistence is selected.
    ///
    /// # Errors
    /// Returns an error when neither an explicit location nor a usable user state
    /// directory can be found.
    pub fn localdb_path(&self) -> anyhow::Result<Option<PathBuf>> {
        if self.storage == Storage::Memory {
            return Ok(None);
        }
        if let Some(path) = &self.localdb_path {
            return Ok(Some(path.clone()));
        }
        let state = std::env::var_os("XDG_STATE_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|path| !path.is_empty())
                    .map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or_else(|| anyhow::anyhow!("set --localdb-path or XDG_STATE_HOME (or HOME)"))?;
        Ok(Some(state.join("basr/sessions.sqlite3")))
    }
}
