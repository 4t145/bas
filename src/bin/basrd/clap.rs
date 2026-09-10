use std::net::SocketAddr;

use anyhow::{Context, Result};
use basr::config::{self, Config};
use clap::Parser;

/// BASR agent service daemon.
#[derive(Clone, Debug, Parser)]
#[command(name = "basrd", version, about, long_about = None)]
pub struct Args {
    /// Address the gRPC service listens on.
    #[arg(long, value_name = "ADDR:PORT")]
    listen: Option<SocketAddr>,

    /// Agent catalog, for example `embedded;opencode=acp:opencode acp`.
    #[arg(long, value_name = "SPEC")]
    agents: Option<String>,

    /// Desktop notification policy: auto, always, or never.
    #[arg(long, value_name = "POLICY")]
    notify: Option<String>,

    /// Replayable events kept per session.
    #[arg(long, value_name = "COUNT")]
    history: Option<usize>,

    /// Tracing backend: auto, stdout, or journald.
    #[arg(long, value_name = "BACKEND")]
    log_backend: Option<String>,

    /// Tracing filter, for example `basr=debug`.
    #[arg(long, value_name = "FILTER")]
    log_filter: Option<String>,
}

impl Args {
    /// Merges command-line flags over the environment-derived configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when a flag carries a malformed value.
    pub fn config(&self) -> Result<Config> {
        let mut config = Config::from_env()?;

        if let Some(listen) = self.listen {
            config.listen = listen;
        }
        if let Some(agents) = &self.agents {
            config.agents = config::parse_agents(agents).context("invalid --agents")?;
        }
        if let Some(notify) = &self.notify {
            config.notify = notify.parse().context("invalid --notify")?;
        }
        if let Some(history) = self.history {
            config.limits.history_capacity = history;
        }

        Ok(config)
    }

    /// Tracing configuration, honoring `--log-*` over the environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the backend name or filter is invalid.
    pub fn tracing(&self) -> Result<crate::tracing::Config> {
        let mut config = crate::tracing::Config::from_env()?;

        if let Some(backend) = &self.log_backend {
            config = config.with_backend(backend.parse().context("invalid --log-backend")?);
        }
        if let Some(filter) = &self.log_filter {
            config = config.with_filter(filter.clone());
        }

        Ok(config)
    }
}
