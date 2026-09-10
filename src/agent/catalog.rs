use std::{collections::BTreeMap, sync::Arc};

use crate::model::AgentId;

use super::{Agent, AgentSpec, Error, Kind, LaunchRequest, driver::Driver};

/// Failure raised while configuring the set of available backends.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("agent {0} is already configured")]
    Duplicate(AgentId),
    #[error("agent {id} is configured as {kind} but has no command line")]
    MissingCommand { id: AgentId, kind: Kind },
    #[error("agent {id} is configured as {kind} but its driver handles {driver_kind}")]
    KindMismatch {
        id: AgentId,
        kind: Kind,
        driver_kind: Kind,
    },
}

/// The set of agent backends the daemon can launch.
#[derive(Default)]
pub struct Catalog {
    entries: BTreeMap<AgentId, Entry>,
}

struct Entry {
    spec: Arc<AgentSpec>,
    driver: Arc<dyn Driver>,
}

impl Catalog {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a catalog holding only the in-process agent.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut catalog = Self::new();
        catalog
            .insert(
                AgentSpec::embedded(),
                Arc::new(super::embedded::Driver::new()),
            )
            .expect("the embedded agent id is unique in an empty catalog");
        catalog
    }

    /// Registers a backend.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError`] when the id is taken, when an external backend
    /// has no command line, or when the driver handles a different kind.
    pub fn insert(&mut self, spec: AgentSpec, driver: Arc<dyn Driver>) -> Result<(), CatalogError> {
        if self.entries.contains_key(&spec.id) {
            return Err(CatalogError::Duplicate(spec.id));
        }

        if spec.kind == Kind::Acp && spec.command.is_none() {
            return Err(CatalogError::MissingCommand {
                id: spec.id.clone(),
                kind: spec.kind,
            });
        }

        if driver.kind() != spec.kind {
            return Err(CatalogError::KindMismatch {
                id: spec.id.clone(),
                kind: spec.kind,
                driver_kind: driver.kind(),
            });
        }

        self.entries.insert(
            spec.id.clone(),
            Entry {
                spec: Arc::new(spec),
                driver,
            },
        );
        Ok(())
    }

    /// Looks up the configuration of one backend.
    #[must_use]
    pub fn spec(&self, id: &AgentId) -> Option<&Arc<AgentSpec>> {
        self.entries.get(id).map(|entry| &entry.spec)
    }

    /// Iterates over every configured backend.
    pub fn specs(&self) -> impl Iterator<Item = &Arc<AgentSpec>> {
        self.entries.values().map(|entry| &entry.spec)
    }

    /// Starts a conversation on the backend named by `request.spec`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the backend is unknown or refuses to
    /// start.
    pub async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn Agent>, Error> {
        let entry = self.entries.get(&request.spec.id).ok_or_else(|| {
            Error::backend(request.spec.id.clone(), "the backend is not configured")
        })?;

        entry.driver.launch(request).await
    }
}
