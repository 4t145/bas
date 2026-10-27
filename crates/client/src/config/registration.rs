#[cfg(feature = "server")]
use super::agent::Kind;
use super::agent::{AgentSpec, ValidationError, validate_agents};
use crate::model::AgentId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io,
    path::{Component, PathBuf},
};

#[cfg(feature = "server")]
use std::{fs, io::Read};

pub const INSTALLATION_FILE: &str = "bas-installation.json";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
#[cfg(feature = "server")]
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentRegistration {
    Custom(AgentSpec),
    Managed(ManagedAgent),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedAgent {
    pub id: AgentId,
    pub installation: InstallationRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationRef {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("invalid installation reference at {path}: {reason}")]
    Reference { path: PathBuf, reason: &'static str },
    #[error("installation I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid installation JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported installation schema version {0}")]
    SchemaVersion(u32),
    #[error("installation agent id {actual} does not match registration {expected}")]
    IdMismatch { expected: AgentId, actual: AgentId },
    #[error("managed agent must be ACP")]
    NotAcp,
}

impl AgentRegistration {
    pub fn id(&self) -> &AgentId {
        match self {
            Self::Custom(spec) => &spec.id,
            Self::Managed(agent) => &agent.id,
        }
    }

    pub fn as_custom(&self) -> Option<&AgentSpec> {
        match self {
            Self::Custom(spec) => Some(spec),
            Self::Managed(_) => None,
        }
    }

    pub fn managed(&self) -> Option<&ManagedAgent> {
        match self {
            Self::Custom(_) => None,
            Self::Managed(agent) => Some(agent),
        }
    }

    /// # Errors
    /// Rejects invalid custom specs and unreadable, changed or invalid installations.
    #[cfg(feature = "server")]
    pub fn resolve(&self) -> Result<AgentSpec, RegistrationError> {
        match self {
            Self::Custom(spec) => {
                validate_agents(std::slice::from_ref(spec))?;
                Ok(spec.clone())
            }
            Self::Managed(agent) => agent.resolve(),
        }
    }
}

impl From<AgentSpec> for AgentRegistration {
    fn from(spec: AgentSpec) -> Self {
        Self::Custom(spec)
    }
}

impl InstallationRef {
    /// Binds the exact bytes before an installer publishes its destination file.
    /// # Errors
    /// Rejects malformed destination paths and oversized manifests.
    pub fn from_bytes(path: PathBuf, bytes: &[u8]) -> Result<Self, RegistrationError> {
        let reference = Self {
            path,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        };
        reference.validate()?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(reference.invalid("manifest exceeds size limit"));
        }
        Ok(reference)
    }

    fn invalid(&self, reason: &'static str) -> RegistrationError {
        RegistrationError::Reference {
            path: self.path.clone(),
            reason,
        }
    }

    fn validate(&self) -> Result<(), RegistrationError> {
        if !self.path.is_absolute()
            || self
                .path
                .components()
                .any(|part| part == Component::ParentDir)
            || self
                .path
                .file_name()
                .is_none_or(|name| name != INSTALLATION_FILE)
        {
            return Err(self.invalid("expected absolute manifest path without parent traversal"));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(self.invalid("expected lowercase SHA-256 digest"));
        }
        Ok(())
    }

    #[cfg(feature = "server")]
    fn io_error(&self, source: io::Error) -> RegistrationError {
        RegistrationError::Io {
            path: self.path.clone(),
            source,
        }
    }

    #[cfg(feature = "server")]
    fn reject_links(&self) -> Result<(), RegistrationError> {
        for ancestor in self.path.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).map_err(|e| self.io_error(e))?;
            if metadata.file_type().is_symlink() {
                return Err(self.invalid("symbolic links are not permitted"));
            }
            if ancestor == self.path && !metadata.is_file() {
                return Err(self.invalid("manifest must be a regular file"));
            }
        }
        Ok(())
    }

    /// # Errors
    /// Rejects malformed references, links, non-files, oversized or changed bytes and I/O failures.
    #[cfg(feature = "server")]
    pub fn read(&self) -> Result<Vec<u8>, RegistrationError> {
        self.validate()?;
        self.reject_links()?;
        let file = fs::File::open(&self.path).map_err(|e| self.io_error(e))?;
        let metadata = file.metadata().map_err(|e| self.io_error(e))?;
        if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
            return Err(self.invalid("manifest must be a regular file within the size limit"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| self.io_error(e))?;
        self.reject_links()?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(self.invalid("manifest exceeds size limit"));
        }
        if format!("{:x}", Sha256::digest(&bytes)) != self.sha256 {
            return Err(self.invalid("manifest SHA-256 mismatch"));
        }
        Ok(bytes)
    }
}

// Metadata belongs to the daemon; shared config only interprets the launch contract.
#[cfg(feature = "server")]
#[derive(Deserialize)]
struct LaunchProjection {
    schema_version: u32,
    spec: AgentSpec,
}

#[cfg(feature = "server")]
impl ManagedAgent {
    /// # Errors
    /// Rejects missing or changed manifests, unsupported schemas, mismatched IDs and invalid ACP specs.
    #[cfg(feature = "server")]
    pub fn resolve(&self) -> Result<AgentSpec, RegistrationError> {
        let bytes = self.installation.read()?;
        let manifest: LaunchProjection = serde_json::from_slice(&bytes)?;
        if manifest.schema_version != SCHEMA_VERSION {
            return Err(RegistrationError::SchemaVersion(manifest.schema_version));
        }
        if manifest.spec.id != self.id {
            return Err(RegistrationError::IdMismatch {
                expected: self.id.clone(),
                actual: manifest.spec.id,
            });
        }
        if manifest.spec.kind != Kind::Acp {
            return Err(RegistrationError::NotAcp);
        }
        validate_agents(std::slice::from_ref(&manifest.spec))?;
        Ok(manifest.spec)
    }
}

/// Validates registration shapes without touching the filesystem.
/// # Errors
/// Rejects empty lists, duplicate/empty IDs, invalid custom specs and malformed references.
pub fn validate_registrations(agents: &[AgentRegistration]) -> Result<(), RegistrationError> {
    if agents.is_empty() {
        return Err(ValidationError("agents must not be empty".into()).into());
    }
    let mut ids = HashSet::new();
    for agent in agents {
        if agent.id().as_str().trim().is_empty() || !ids.insert(agent.id()) {
            return Err(
                ValidationError(format!("empty or duplicate agent id {}", agent.id())).into(),
            );
        }
        match agent {
            AgentRegistration::Custom(spec) => validate_agents(std::slice::from_ref(spec))?,
            AgentRegistration::Managed(agent) => agent.installation.validate()?,
        }
    }
    Ok(())
}

/// # Errors
/// Rejects invalid registrations or any installation that cannot be resolved safely.
#[cfg(feature = "server")]
pub fn resolve_agents(agents: &[AgentRegistration]) -> Result<Vec<AgentSpec>, RegistrationError> {
    validate_registrations(agents)?;
    agents.iter().map(AgentRegistration::resolve).collect()
}

#[cfg(all(test, feature = "server"))]
mod tests;
