//! Immutable version descriptors and the single settings activation boundary.

use super::{Error, Record, artifact, authentication, registry};
use anyhow::{Context, Result};
use bas::config::{
    AgentRegistration, AgentSpec, DaemonDocument as Document, DaemonStore as Store,
    INSTALLATION_FILE, InstallationRef, ManagedAgent,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Component};

const SCHEMA_VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
struct Descriptor {
    schema_version: u32,
    #[serde(flatten)]
    record: Record,
}

#[derive(Debug, thiserror::Error)]
enum DescriptorError {
    #[error("managed installation metadata has an invalid identity or directory")]
    Identity,
    #[error("managed installation differs from the selected launch configuration")]
    Spec,
    #[error("managed installation has no trusted launch boundary")]
    Launch,
}

/// # Errors
/// Rejects missing, altered, unsupported, or incorrectly bound descriptors.
pub(crate) fn load_record(managed: &ManagedAgent) -> Result<Record> {
    let bytes = managed.installation.read()?;
    let descriptor: Descriptor =
        serde_json::from_slice(&bytes).context("invalid managed installation metadata")?;
    if descriptor.schema_version != SCHEMA_VERSION {
        return Err(Error::RecordsVersion.into());
    }
    let record = descriptor.record;
    registry::validate_component(managed.id.as_str())?;
    if record.manifest.id != managed.id.as_str()
        || record.spec.id != managed.id
        || record.spec != managed.resolve()?
        || managed.installation.path.file_name() != Some(INSTALLATION_FILE.as_ref())
        || managed.installation.path.parent() != Some(record.directory.as_path())
        || !record.directory.is_absolute()
        || record
            .directory
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(DescriptorError::Identity.into());
    }
    artifact::reject_directory_links(&record.directory)?;
    Ok(record)
}

/// Resolves the full daemon-owned descriptor, not just its shared launch projection.
///
/// # Errors
/// Rejects invalid or incomplete installation metadata.
pub(crate) fn resolve(managed: &ManagedAgent) -> Result<AgentSpec> {
    Ok(load_record(managed)?.spec)
}

/// # Errors
/// Rejects stale, missing or modified managed descriptors before starting a process.
pub(crate) fn validate_spec(managed: &ManagedAgent, spec: &AgentSpec) -> Result<()> {
    if resolve(managed)? != *spec {
        return Err(DescriptorError::Spec.into());
    }
    Ok(())
}

/// # Errors
/// Rejects any custom registration using the requested identity.
pub(super) fn check_identity(id: &str, agents: &[AgentRegistration]) -> Result<()> {
    if agents
        .iter()
        .any(|agent| agent.id().as_str() == id && agent.as_custom().is_some())
    {
        return Err(Error::RegistrationConflict(id.into()).into());
    }
    Ok(())
}

/// # Errors
/// Reports filesystem or descriptor failures before a version can be activated.
pub(super) fn finalize(staging: tempfile::TempDir, mut record: Record) -> Result<ManagedAgent> {
    // Runtime launch boundaries can depend on files at their final absolute paths.
    fs::rename(staging.path(), &record.directory)?;
    let mut unfinished = UnfinishedDirectory(Some(record.directory.clone()));
    record.launch =
        Some(authentication::LaunchBoundary::from_record(&record).ok_or(DescriptorError::Launch)?);
    let path = record.directory.join(INSTALLATION_FILE);
    let id = record.spec.id.clone();
    artifact::atomic_json(
        &path,
        &Descriptor {
            schema_version: SCHEMA_VERSION,
            record,
        },
    )?;
    let installation = InstallationRef::from_bytes(path.clone(), &fs::read(&path)?)?;
    let managed = ManagedAgent { id, installation };
    load_record(&managed)?;
    // After this point a settings commit may have succeeded even when it reports an error.
    // Retain the complete version independently of activation's outcome.
    unfinished.0 = None;
    Ok(managed)
}

/// # Errors
/// Rejects custom collisions, invalid descriptors, and stale or unwritable settings.
pub(super) fn activate(
    store: &Store,
    document: &mut Document,
    managed: ManagedAgent,
) -> Result<()> {
    load_record(&managed)?;
    let mut agents = document.settings.agents.clone();
    check_identity(managed.id.as_str(), &agents)?;
    if let Some(existing) = agents.iter_mut().find(|agent| agent.id() == &managed.id) {
        *existing = AgentRegistration::Managed(managed);
    } else {
        agents.push(AgentRegistration::Managed(managed));
    }
    store.save_agents(document, agents)?;
    Ok(())
}

struct UnfinishedDirectory(Option<std::path::PathBuf>);

impl Drop for UnfinishedDirectory {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}
