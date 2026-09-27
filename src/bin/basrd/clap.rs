use std::{
    net::SocketAddr,
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result};
use basr::service::DEFAULT_ADDRESS;
use clap::Parser;

use crate::{
    agent::PermissionPolicy,
    config::{self, Config, Notify, Storage},
    session::{DEFAULT_BUFFER, DEFAULT_HISTORY, DEFAULT_STARTUP_TIMEOUT, Limits},
};

/// BASR agent service daemon.
#[derive(Clone, Debug, Parser)]
#[command(name = "basrd", version, about, long_about = None)]
pub struct Args {
    /// Address the gRPC service listens on.
    #[arg(long, env = config::LISTEN_ENV, default_value = DEFAULT_ADDRESS, value_name = "ADDR:PORT")]
    listen: SocketAddr,
    /// Agent catalog, for example `embedded;opencode=acp:opencode acp`.
    #[arg(long, env = config::AGENTS_ENV, value_name = "SPEC")]
    agents: Option<String>,
    /// Explicit legacy JSON agent catalog.
    #[arg(long, env = config::AGENTS_FILE_ENV, value_name = "PATH")]
    agents_file: Option<PathBuf>,
    /// Unified settings file; defaults to the user's basr/settings.json.
    #[arg(long, env = "BASR_SETTINGS_FILE", value_name = "PATH")]
    settings_file: Option<PathBuf>,
    /// Desktop notification policy: auto, always (required), or never.
    #[arg(long, env = config::NOTIFY_ENV, default_value = "auto")]
    notify: Notify,
    /// Replayable events kept per session; must be greater than zero.
    #[arg(long, env = config::HISTORY_ENV, default_value_t = NonZeroUsize::new(DEFAULT_HISTORY).expect("nonzero default"))]
    history: NonZeroUsize,
    /// Session storage backend. Defaults to local SQLite persistence.
    #[arg(long, env = config::STORAGE_ENV, value_enum, default_value = "localdb")]
    storage: Storage,
    /// SQLite file location for `--storage localdb`.
    #[arg(long, env = config::LOCALDB_PATH_ENV, value_name = "PATH")]
    localdb_path: Option<PathBuf>,
    /// Backend queue and live broadcast capacities; must be greater than zero.
    #[arg(long, env = config::BUFFER_ENV, default_value_t = NonZeroUsize::new(DEFAULT_BUFFER).expect("nonzero default"))]
    buffer: NonZeroUsize,
    /// Maximum backend startup time in seconds.
    #[arg(long, env = config::STARTUP_TIMEOUT_ENV, default_value_t = NonZeroU64::new(DEFAULT_STARTUP_TIMEOUT).expect("nonzero default"))]
    startup_timeout: NonZeroU64,
    /// Fixed response to backend permission requests.
    #[arg(long, env = config::PERMISSIONS_ENV, value_enum, default_value = "deny")]
    permissions: PermissionPolicy,
    /// Tracing backend: auto, stdout, or journald.
    #[arg(long, env = crate::tracing::BACKEND_ENV, default_value = "auto")]
    log_backend: crate::tracing::Backend,
    /// Tracing filter, for example `basrd=debug`.
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_filter: String,
}

impl Args {
    /// Clap selects command-line values before environment values are parsed.
    ///
    /// # Errors
    /// Returns an error if the selected agent catalog is invalid or empty.
    pub fn config(&self) -> Result<Config> {
        let (agents, catalog_source) = if let Some(agents) = &self.agents {
            (
                config::parse_agents(agents).context("invalid agent catalog")?,
                None,
            )
        } else {
            let mut source = match &self.agents_file {
                Some(path) => config::CatalogSource::legacy(path.clone()),
                None => config::CatalogSource::settings(self.settings_file.clone())?,
            };
            let agents = source.load()?;
            (agents, Some(source))
        };
        anyhow::ensure!(!agents.is_empty(), "at least one agent must be configured");
        anyhow::ensure!(
            self.storage == Storage::Localdb || self.localdb_path.is_none(),
            "--localdb-path requires --storage localdb"
        );
        Ok(Config {
            listen: self.listen,
            notify: self.notify,
            agents,
            catalog_source,
            permissions: self.permissions,
            storage: self.storage,
            localdb_path: self.localdb_path.clone(),
            limits: Limits {
                history_capacity: self.history,
                live_capacity: self.buffer,
                startup_timeout: Duration::from_secs(self.startup_timeout.get()),
            },
        })
    }

    pub fn tracing(&self) -> crate::tracing::Config {
        crate::tracing::Config::new(self.log_backend).with_filter(&self.log_filter)
    }
}
