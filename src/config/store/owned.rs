use super::*;
use crate::config::{Daemon, FrontendSettings};

#[derive(Clone, Debug)]
pub struct FrontendStore(Store);

#[derive(Clone, Debug)]
pub struct FrontendDocument {
    pub settings: FrontendSettings,
    inner: Document,
}

impl From<Document> for FrontendDocument {
    fn from(inner: Document) -> Self {
        Self {
            settings: FrontendSettings {
                appearance: inner.settings.appearance.clone(),
                frontend: inner.settings.frontend.clone(),
            },
            inner,
        }
    }
}

impl FrontendDocument {
    pub fn source_paths(&self) -> &[PathBuf] {
        self.inner.source_paths()
    }
}

impl FrontendStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self(Store::frontend(path))
    }
    pub fn at(path: PathBuf) -> Self {
        Self::new(Some(path))
    }
    pub fn path(&self) -> &Path {
        self.0.path()
    }

    /// # Errors
    /// Rejects invalid frontend preferences without decoding legacy daemon fields.
    pub fn load(&self) -> Result<FrontendDocument, Error> {
        self.0.load().map(Into::into)
    }

    /// # Errors
    /// Preserves conflict, atomic-write and symlink checks for the appearance section.
    pub fn save_appearance(
        &self,
        document: &mut FrontendDocument,
        value: Appearance,
    ) -> Result<(), Error> {
        self.0.save_appearance(&mut document.inner, value)?;
        *document = document.inner.clone().into();
        Ok(())
    }

    /// # Errors
    /// Preserves conflict, atomic-write and symlink checks for connection preferences.
    pub fn save_frontend(
        &self,
        document: &mut FrontendDocument,
        value: Frontend,
    ) -> Result<(), Error> {
        self.0.save_frontend(&mut document.inner, value)?;
        *document = document.inner.clone().into();
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct DaemonStore(Store);

#[derive(Clone, Debug)]
pub struct DaemonDocument {
    pub settings: Daemon,
    inner: Document,
}

impl From<Document> for DaemonDocument {
    fn from(inner: Document) -> Self {
        Self {
            settings: inner.settings.daemon.clone(),
            inner,
        }
    }
}

impl DaemonDocument {
    pub fn source_paths(&self) -> &[PathBuf] {
        self.inner.source_paths()
    }
}

impl DaemonStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self(Store::daemon(path))
    }
    pub fn at(path: PathBuf) -> Self {
        Self::new(Some(path))
    }
    pub fn path(&self) -> &Path {
        self.0.path()
    }

    /// # Errors
    /// Rejects invalid backend settings without decoding frontend preferences.
    pub fn load(&self) -> Result<DaemonDocument, Error> {
        self.0.load().map(Into::into)
    }

    /// # Errors
    /// Validates registrations and ACP policy together, preserving concurrent policy edits.
    pub fn save_agents(
        &self,
        document: &mut DaemonDocument,
        value: Vec<AgentRegistration>,
    ) -> Result<(), Error> {
        self.0.save_agents(&mut document.inner, value)?;
        *document = document.inner.clone().into();
        Ok(())
    }

    /// # Errors
    /// Rejects stale policy/catalog snapshots, invalid targets and failed writes.
    pub fn save_acp(
        &self,
        document: &mut DaemonDocument,
        value: crate::config::AcpLifecycle,
    ) -> Result<(), Error> {
        self.0.save(&mut document.inner, Section::Acp(value))?;
        *document = document.inner.clone().into();
        Ok(())
    }
}
