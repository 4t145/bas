use std::{
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result};
use bas::service::{DEFAULT_TARGET, Target};
use clap::Parser;

use crate::{
    agent::PermissionPolicy,
    config::{self, Config, Notify, Storage},
    session::{DEFAULT_BUFFER, DEFAULT_HISTORY, DEFAULT_STARTUP_TIMEOUT, Limits},
};

/// Bas agent service daemon.
#[derive(Clone, Debug, Parser)]
#[command(name = "bas", version, about, long_about = None)]
pub struct Args {
    /// Repeat for multiple endpoints: local, tcp://HOST:PORT, or mtls://HOST:PORT.
    #[arg(long, env = config::LISTEN_ENV, default_value = DEFAULT_TARGET, value_delimiter = ';', value_name = "TARGET")]
    listen: Vec<Target>,
    /// PEM server certificate chain for mtls:// listeners.
    #[arg(long, requires_all = ["tls_key", "tls_client_ca"])]
    tls_certificate: Option<PathBuf>,
    /// PEM server private key (file reference only).
    #[arg(long, requires_all = ["tls_certificate", "tls_client_ca"])]
    tls_key: Option<PathBuf>,
    /// PEM CA bundle trusted to issue client certificates.
    #[arg(long, requires_all = ["tls_certificate", "tls_key"])]
    tls_client_ca: Option<PathBuf>,
    /// Agent catalog, for example `embedded;opencode=acp:opencode acp`.
    #[arg(long, env = config::AGENTS_ENV, value_name = "SPEC")]
    agents: Option<String>,
    /// Explicit legacy JSON agent catalog.
    #[arg(long, env = config::AGENTS_FILE_ENV, value_name = "PATH")]
    agents_file: Option<PathBuf>,
    /// Backend settings; defaults to the user's bas/daemon.json.
    #[arg(long, env = "BAS_DAEMON_SETTINGS_FILE", value_name = "PATH")]
    daemon_settings_file: Option<PathBuf>,
    /// Deprecated explicit backend settings path; use --daemon-settings-file.
    #[arg(long, value_name = "PATH", conflicts_with = "daemon_settings_file")]
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
    /// Tracing filter, for example `bas=debug`.
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_filter: String,
}

impl Args {
    /// Clap selects command-line values before environment values are parsed.
    ///
    /// # Errors
    /// Returns an error if the selected agent catalog is invalid or empty.
    pub fn config(&self) -> Result<Config> {
        let mut source = config::CatalogSource::settings(
            self.daemon_settings_file
                .clone()
                .or_else(|| self.settings_file.clone()),
        )?;
        if let Some(agents) = &self.agents {
            source = source.with_agents(
                config::parse_agents(agents)
                    .context("invalid agent catalog")?
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            );
        } else if let Some(path) = &self.agents_file {
            source = source.with_legacy_agents(path.clone());
        }
        let bas::config::Daemon { agents, acp } = source.load_configuration()?;
        let catalog_source = Some(source);
        anyhow::ensure!(!agents.is_empty(), "at least one agent must be configured");
        anyhow::ensure!(
            self.storage == Storage::Localdb || self.localdb_path.is_none(),
            "--localdb-path requires --storage localdb"
        );
        let tls = match (&self.tls_certificate, &self.tls_key, &self.tls_client_ca) {
            (Some(certificate), Some(private_key), Some(client_ca)) => {
                Some(bas::service::tls::ServerTls {
                    certificate: certificate.clone(),
                    private_key: private_key.clone(),
                    client_ca: client_ca.clone(),
                })
            }
            (None, None, None) => None,
            _ => anyhow::bail!("mTLS requires --tls-certificate, --tls-key and --tls-client-ca"),
        };
        anyhow::ensure!(
            self.listen
                .iter()
                .any(|target| matches!(target, Target::Mtls(_)))
                == tls.is_some(),
            "mtls:// listeners require TLS files; TLS files require an mtls:// listener"
        );
        Ok(Config {
            listen: self.listen.clone(),
            tls,
            notify: self.notify,
            agents,
            acp,
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
