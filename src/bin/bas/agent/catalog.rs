use std::{collections::BTreeMap, sync::Arc};

use bas::{
    config::{AgentRegistration, ManagedAgent, RegistrationError, validate_registrations},
    model::AgentId,
};

use super::{Agent, AgentSpec, Error, Kind, LaunchRequest, acp, driver::Driver, embedded};

/// Failure raised while configuring the set of available backends.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Registration(#[from] RegistrationError),
    #[error("cannot load managed installation: {0}")]
    Installation(#[source] anyhow::Error),
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
    registration: AgentRegistration,
    spec: Arc<AgentSpec>,
    driver: Arc<dyn Driver>,
}

impl Catalog {
    pub fn invalidate_pool(&self, id: &AgentId) {
        if let Some(entry) = self.entries.get(id) {
            entry.driver.invalidate_pool();
        }
    }

    /// Configures process prewarming without waiting for initialization.
    pub fn maintain_pools(
        &self,
        targets: &BTreeMap<AgentId, usize>,
        startup_timeout: std::time::Duration,
    ) {
        for (id, entry) in &self.entries {
            let target = if self.validate(&entry.spec).is_ok() {
                targets.get(id).copied().unwrap_or(0)
            } else {
                0
            };
            entry
                .driver
                .maintain_pool(entry.spec.clone(), target, startup_timeout);
        }
    }

    pub async fn shutdown_pools(&self) {
        futures::future::join_all(
            self.entries
                .values()
                .map(|entry| entry.driver.shutdown_pool()),
        )
        .await;
    }

    /// Builds the catalog using the configured backend drivers.
    /// # Errors
    /// Returns an error for invalid or duplicate backend specifications.
    #[cfg(test)]
    pub fn from_specs(specs: impl IntoIterator<Item = AgentSpec>) -> Result<Self, CatalogError> {
        Self::from_registrations(specs.into_iter().map(AgentRegistration::from).collect())
    }

    /// Resolves a profile's registrations before publishing any part of its catalog.
    ///
    /// # Errors
    /// Rejects invalid registrations or missing, modified and invalid installation manifests.
    pub fn from_registrations(registrations: Vec<AgentRegistration>) -> Result<Self, CatalogError> {
        validate_registrations(&registrations)?;
        let mut catalog = Catalog::new();

        for registration in registrations {
            let spec = match &registration {
                AgentRegistration::Managed(managed) => {
                    crate::management::installation::resolve(managed)
                        .map_err(CatalogError::Installation)?
                }
                AgentRegistration::Custom(_) => registration.resolve()?,
            };
            let driver: Arc<dyn Driver> = match spec.kind {
                Kind::Embedded => Arc::new(embedded::Driver::new()),
                Kind::Acp => Arc::new(acp::Driver::new()),
            };

            catalog.insert_registration(registration, spec, driver)?;
        }

        Ok(catalog)
    }

    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a catalog holding only the in-process agent.
    #[must_use]
    #[cfg(test)]
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
    #[cfg(test)]
    pub fn insert(&mut self, spec: AgentSpec, driver: Arc<dyn Driver>) -> Result<(), CatalogError> {
        self.insert_registration(spec.clone().into(), spec, driver)
    }

    fn insert_registration(
        &mut self,
        registration: AgentRegistration,
        spec: AgentSpec,
        driver: Arc<dyn Driver>,
    ) -> Result<(), CatalogError> {
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
                registration,
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

    pub fn registrations(&self) -> impl Iterator<Item = &AgentRegistration> {
        self.entries.values().map(|entry| &entry.registration)
    }

    pub fn managed(&self, id: &AgentId) -> Option<&ManagedAgent> {
        self.entries.get(id)?.registration.managed()
    }

    /// A retained last-valid catalog does not authorize a new process from a broken reference.
    ///
    /// # Errors
    /// Rejects unconfigured or stale specs and missing, modified or invalid managed descriptors.
    pub fn validate(&self, spec: &AgentSpec) -> Result<(), Error> {
        let entry = self
            .entries
            .get(&spec.id)
            .ok_or_else(|| Error::backend(spec.id.clone(), "the backend is not configured"))?;
        if entry.spec.as_ref() != spec {
            return Err(Error::backend(
                spec.id.clone(),
                "the backend configuration changed",
            ));
        }
        if let Some(managed) = entry.registration.managed() {
            crate::management::installation::validate_spec(managed, spec)
                .map_err(|error| Error::backend(spec.id.clone(), error.to_string()))?;
        }
        Ok(())
    }

    /// Starts a conversation on the backend named by `request.spec`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the backend is unknown or refuses to
    /// start.
    pub async fn launch(&self, request: LaunchRequest) -> Result<Box<dyn Agent>, Error> {
        self.validate(&request.spec)?;
        let entry = self
            .entries
            .get(&request.spec.id)
            .expect("validated backend");
        entry.driver.launch(request).await
    }

    /// Restores a backend using its private session identifier, never falling
    /// back to a new conversation when the backend cannot load it.
    ///
    /// # Errors
    /// Returns an error if the backend is unavailable or rejects recovery.
    pub async fn restore(
        &self,
        request: LaunchRequest,
        backend_session_id: String,
    ) -> Result<Box<dyn Agent>, Error> {
        self.validate(&request.spec)?;
        let entry = self
            .entries
            .get(&request.spec.id)
            .expect("validated backend");
        entry.driver.restore(request, backend_session_id).await
    }
}
