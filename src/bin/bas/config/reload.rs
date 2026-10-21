//! Atomic catalog and ACP lifecycle reloads. Other flags remain startup configuration.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;

use crate::session::Registry;
use bas::config::AgentRegistration;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogSource {
    path: PathBuf,
    explicit: bool,
    agents_override: Option<AgentsOverride>,
    /// Once a file has been observed it must not disappear into defaults.
    required: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AgentsOverride {
    Fixed(Vec<AgentRegistration>),
    Legacy(PathBuf),
}

impl CatalogSource {
    /// Settings-backed catalogs can persist managed installations.
    pub fn settings_store(&self) -> Option<bas::config::Store> {
        self.agents_override.is_none().then(|| {
            if self.explicit {
                bas::config::Store::at(self.path.clone())
            } else {
                bas::config::Store::new(None)
            }
        })
    }

    pub fn settings(path: Option<PathBuf>) -> Result<Self> {
        let explicit = path.is_some();
        let store = bas::config::Store::new(path);
        Ok(Self {
            path: store.path().to_owned(),
            explicit,
            agents_override: None,
            required: None,
        })
    }

    #[cfg(test)]
    pub fn load(&mut self) -> Result<Vec<AgentRegistration>> {
        Ok(self.load_configuration()?.agents)
    }

    pub fn with_agents(mut self, agents: Vec<AgentRegistration>) -> Self {
        self.agents_override = Some(AgentsOverride::Fixed(agents));
        self
    }

    pub fn with_legacy_agents(mut self, path: PathBuf) -> Self {
        self.agents_override = Some(AgentsOverride::Legacy(path));
        self
    }

    /// # Errors
    /// Rejects invalid or disappeared sources before publishing either section.
    pub fn load_configuration(&mut self) -> Result<bas::config::Daemon> {
        let mut daemon = self.load_settings()?;
        match &self.agents_override {
            Some(AgentsOverride::Fixed(agents)) => daemon.agents = agents.clone(),
            Some(AgentsOverride::Legacy(path)) => {
                daemon.agents = super::read_agents(path)?
                    .into_iter()
                    .map(Into::into)
                    .collect();
            }
            None => {}
        }
        bas::config::validate_registrations(&daemon.agents)?;
        daemon.acp.validate(&daemon.agents)?;
        Ok(daemon)
    }

    fn load_settings(&mut self) -> Result<bas::config::Daemon> {
        // Latch the highest-priority file even when it is temporarily invalid.
        // A removed settings file must not silently resurrect the legacy catalog.
        match std::fs::metadata(&self.path) {
            Ok(metadata) => {
                anyhow::ensure!(
                    metadata.is_file(),
                    "settings path is not a regular file: {}",
                    self.path.display()
                );
                self.required = Some(self.path.clone());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !self.explicit => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("cannot read settings {}", self.path.display()));
            }
        }
        if self.required.is_none() && !self.explicit {
            let legacy = self.path.with_file_name("agents.json");
            match std::fs::metadata(&legacy) {
                Ok(_) => self.required = Some(legacy),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("cannot inspect legacy agent catalog"),
            }
        }
        if let Some(required) = &self.required {
            let metadata = std::fs::metadata(required)
                .with_context(|| format!("catalog source disappeared: {}", required.display()))?;
            anyhow::ensure!(
                metadata.is_file(),
                "catalog source is not a regular file: {}",
                required.display()
            );
        }
        // Explicit settings are isolated: missing files are errors, not import/defaults.
        let store = if self.explicit || self.required.as_ref() == Some(&self.path) {
            bas::config::Store::at(self.path.clone())
        } else {
            bas::config::Store::new(None)
        };
        let document = store.load()?;
        // A file may have appeared between inspection and load. Latch the
        // actual successful source as well, not only the metadata observation.
        if document.source_paths().contains(&self.path) {
            self.required = Some(self.path.clone());
        } else if self.required.is_none() {
            let legacy = self.path.with_file_name("agents.json");
            if document.source_paths().contains(&legacy) {
                self.required = Some(legacy);
            }
        }
        if let Some(required) = &self.required {
            anyhow::ensure!(
                document.source_paths().contains(required),
                "catalog source disappeared: {}",
                required.display()
            );
        }
        Ok(document.settings.daemon)
    }
}

/// Every publication is fully validated; failures preserve catalog and policy.
pub fn watch_catalog(
    mut source: CatalogSource,
    mut current: Vec<AgentRegistration>,
    registry: Arc<Registry>,
    stopping: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let mut current_policy = registry.acp_policy();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_error = None;
        loop {
            tokio::select! {
                biased;
                () = stopping.cancelled() => break,
                _ = interval.tick() => {}
            }
            let result = registry.reload_configuration(|| source.load_configuration());
            match result {
                Ok(bas::config::Daemon { agents, acp }) => {
                    last_error = None;
                    if agents != current || acp != current_policy {
                        current = agents;
                        current_policy = acp;
                        ::tracing::info!(
                            agents = current.len(),
                            "agent catalog and ACP policy reloaded"
                        );
                    }
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    if last_error.as_ref() != Some(&error) {
                        // Parser errors can contain configuration values (including secrets).
                        // Keep their detail only for deduplication, never in the log.
                        ::tracing::warn!(path = %source.path.display(), "cannot reload agent catalog or ACP policy; keeping previous configuration");
                        last_error = Some(error);
                    }
                }
            }
        }
    })
}
