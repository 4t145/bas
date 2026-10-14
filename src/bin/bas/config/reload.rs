//! Catalog-only reloads. Other daemon flags remain startup configuration.

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
    settings: bool,
    explicit: bool,
    /// Once a file has been observed it must not disappear into defaults.
    required: Option<PathBuf>,
}

impl CatalogSource {
    /// Settings-backed catalogs can persist managed installations.
    pub fn settings_store(&self) -> Option<bas::config::Store> {
        self.settings.then(|| {
            if self.explicit {
                bas::config::Store::at(self.path.clone())
            } else {
                bas::config::Store::new(None)
            }
        })
    }

    pub fn legacy(path: PathBuf) -> Self {
        Self {
            path,
            settings: false,
            explicit: true,
            required: None,
        }
    }

    pub fn settings(path: Option<PathBuf>) -> Result<Self> {
        let explicit = path.is_some();
        let store = bas::config::Store::new(path);
        Ok(Self {
            path: store.path().to_owned(),
            settings: true,
            explicit,
            required: None,
        })
    }

    pub fn load(&mut self) -> Result<Vec<AgentRegistration>> {
        if !self.settings {
            return Ok(super::read_agents(&self.path)?
                .into_iter()
                .map(Into::into)
                .collect());
        }
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
        Ok(document.settings.daemon.agents)
    }
}

/// Every publication is a fully validated catalog; failures preserve the last snapshot.
pub fn watch_catalog(
    mut source: CatalogSource,
    mut current: Vec<AgentRegistration>,
    registry: Arc<Registry>,
    stopping: CancellationToken,
) -> tokio::task::JoinHandle<()> {
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
            let result = registry.reload_catalog(|| source.load());
            match result {
                Ok(agents) => {
                    last_error = None;
                    if agents != current {
                        current = agents;
                        ::tracing::info!(agents = current.len(), "agent catalog reloaded");
                    }
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    if last_error.as_ref() != Some(&error) {
                        // Parser errors can contain configuration values (including secrets).
                        // Keep their detail only for deduplication, never in the log.
                        ::tracing::warn!(path = %source.path.display(), "cannot reload agent catalog; keeping previous catalog");
                        last_error = Some(error);
                    }
                }
            }
        }
    })
}
