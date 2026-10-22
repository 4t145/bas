use super::{
    AgentRegistration, AgentSpec, Appearance, Frontend, Settings, validate_agents,
    validate_registrations,
};
use serde::de::DeserializeOwned;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

mod owned;
mod scoped;
pub use owned::{DaemonDocument, DaemonStore, FrontendDocument, FrontendStore};

#[cfg(test)]
mod tests;

struct WriteLock(fs::File);

impl Drop for WriteLock {
    fn drop(&mut self) {
        // A concurrent subprocess spawn can briefly inherit this file descriptor.
        // Unlock explicitly: closing only our copy can leave its inherited copy locked.
        let _ = self.0.unlock();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("settings I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid settings JSON at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid settings at {path}: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("settings {section} changed externally at {path}; reload before saving")]
    Conflict {
        path: PathBuf,
        section: &'static str,
    },
    #[error("settings are being saved by another writer: {path}")]
    Busy { path: PathBuf },
    #[error("refusing symbolic link at {path}")]
    Symlink { path: PathBuf },
}
#[derive(Clone, Debug)]
pub struct Document {
    pub settings: Settings,
    baseline: Settings,
    sources: Vec<PathBuf>,
}
impl Document {
    pub fn source_paths(&self) -> &[PathBuf] {
        &self.sources
    }
}
#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
    import_legacy: bool,
    scope: Scope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Unified,
    Frontend,
    Daemon,
}
#[derive(PartialEq)]
struct Snapshot {
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}
struct Loaded {
    document: Document,
    snapshot: Snapshot,
    raw: serde_json::Value,
}
enum Section {
    Appearance(Appearance),
    Agents(Vec<AgentRegistration>),
    Acp(super::AcpLifecycle),
    Frontend(Frontend),
}
impl Store {
    pub fn new(path: Option<PathBuf>) -> Self {
        match path {
            Some(path) => Self::at(path),
            None => Self {
                path: super::paths::config_root()
                    .unwrap_or_else(|| PathBuf::from(".config"))
                    .join("bas/settings.json"),
                import_legacy: true,
                scope: Scope::Unified,
            },
        }
    }
    /// An isolated path never consults legacy configuration.
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            import_legacy: false,
            scope: Scope::Unified,
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// # Errors
    /// Rejects unreadable, malformed or invalid current/legacy files.
    /// Managed symlinks can be read, but are never replaced by save operations.
    pub fn load(&self) -> Result<Document, Error> {
        Ok(self.read()?.document)
    }
    /// # Errors
    /// Rejects conflicts, busy locks, links, invalid documents and I/O failures.
    pub fn save_appearance(
        &self,
        document: &mut Document,
        appearance: Appearance,
    ) -> Result<(), Error> {
        self.save(document, Section::Appearance(appearance))
    }
    /// # Errors
    /// Rejects conflicts, busy locks, links, invalid documents and I/O failures.
    pub fn save_agents(
        &self,
        document: &mut Document,
        agents: Vec<AgentRegistration>,
    ) -> Result<(), Error> {
        self.save(document, Section::Agents(agents))
    }
    /// # Errors
    /// Rejects conflicts, busy locks, links, invalid documents and I/O failures.
    pub fn save_frontend(&self, document: &mut Document, frontend: Frontend) -> Result<(), Error> {
        self.save(document, Section::Frontend(frontend))
    }
    fn read(&self) -> Result<Loaded, Error> {
        if self.scope != Scope::Unified {
            return self.read_scoped();
        }
        let mut snapshot = Snapshot { files: Vec::new() };
        let current = read_optional(&self.path)?;
        snapshot.files.push((self.path.clone(), current.clone()));
        let mut settings = Settings::default();
        if let Some(bytes) = current {
            settings = parse(&self.path, &bytes)?;
            validate(&self.path, &settings)?;
        } else if self.import_legacy {
            let parent = self.parent();
            let appearance_path = parent.join("appearance.json");
            let agents_path = parent.join("agents.json");
            let appearance = read_optional(&appearance_path)?;
            let agents = read_optional(&agents_path)?;
            if let Some(bytes) = &appearance {
                settings.appearance = parse(&appearance_path, bytes)?;
                settings
                    .appearance
                    .validate()
                    .map_err(|e| invalid(&appearance_path, e))?;
            }
            if let Some(bytes) = &agents {
                let legacy: Vec<AgentSpec> = parse(&agents_path, bytes)?;
                validate_agents(&legacy).map_err(|e| invalid(&agents_path, e))?;
                settings.daemon.agents = legacy.into_iter().map(Into::into).collect();
            }
            snapshot.files.push((appearance_path, appearance));
            snapshot.files.push((agents_path, agents));
        }
        let sources = snapshot
            .files
            .iter()
            .filter(|(_, bytes)| bytes.is_some())
            .map(|(path, _)| path.clone())
            .collect();
        Ok(Loaded {
            document: Document {
                baseline: settings.clone(),
                settings,
                sources,
            },
            snapshot,
            raw: serde_json::Value::Null,
        })
    }
    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }
    fn lock_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        name.into()
    }
    fn save(&self, document: &mut Document, section: Section) -> Result<(), Error> {
        if matches!(
            (&section, self.scope),
            (Section::Agents(_) | Section::Acp(_), Scope::Frontend)
                | (Section::Appearance(_) | Section::Frontend(_), Scope::Daemon)
        ) {
            return Err(invalid(
                &self.path,
                "configuration section belongs to the other application",
            ));
        }
        reject_links(&self.path)?;
        fs::create_dir_all(self.parent()).map_err(|e| io_error(self.parent(), e))?;
        let lock_path = self.lock_path();
        reject_links(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| io_error(&lock_path, e))?;
        lock.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => Error::Busy {
                path: self.path.clone(),
            },
            std::fs::TryLockError::Error(e) => io_error(&lock_path, e),
        })?;
        let _lock = WriteLock(lock);
        let loaded = self.read()?;
        let mut settings = loaded.document.settings;
        let name = match section {
            Section::Appearance(appearance) => {
                if settings.appearance != document.baseline.appearance {
                    return Err(self.conflict("appearance"));
                }
                settings.appearance = appearance;
                "appearance"
            }
            Section::Agents(agents) => {
                if settings.daemon.agents != document.baseline.daemon.agents {
                    return Err(self.conflict("daemon.agents"));
                }
                settings.daemon.agents = agents;
                "daemon.agents"
            }
            Section::Acp(policy) => {
                if settings.daemon != document.baseline.daemon {
                    return Err(self.conflict("daemon"));
                }
                settings.daemon.acp = policy;
                "daemon.acp"
            }
            Section::Frontend(frontend) => {
                if settings.frontend != document.baseline.frontend {
                    return Err(self.conflict("frontend"));
                }
                settings.frontend = frontend;
                "frontend"
            }
        };
        self.validate_scoped(&settings)?;
        let value = self.encode_scoped(&settings, loaded.raw)?;
        let bytes = serde_json::to_vec_pretty(&value).map_err(|source| Error::Json {
            path: self.path.clone(),
            source,
        })?;
        let mut temp =
            NamedTempFile::new_in(self.parent()).map_err(|e| io_error(self.parent(), e))?;
        temp.write_all(&bytes)
            .and_then(|_| temp.write_all(b"\n"))
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| io_error(&self.path, e))?;
        // Catch non-cooperating editors as well as changes to imported source files.
        for (path, original) in &loaded.snapshot.files {
            if read_optional(path)? != *original {
                return Err(self.conflict(name));
            }
        }
        reject_links(&self.path)?;
        temp.persist(&self.path)
            .map_err(|e| io_error(&self.path, e.error))?;
        *document = Document {
            baseline: settings.clone(),
            settings,
            sources: vec![self.path.clone()],
        };
        Ok(())
    }
    fn conflict(&self, section: &'static str) -> Error {
        Error::Conflict {
            path: self.path.clone(),
            section,
        }
    }
}
fn io_error(path: &Path, source: io::Error) -> Error {
    Error::Io {
        path: path.into(),
        source,
    }
}
fn invalid(path: &Path, reason: impl std::fmt::Display) -> Error {
    Error::Invalid {
        path: path.into(),
        reason: reason.to_string(),
    }
}
fn parse<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|source| Error::Json {
        path: path.into(),
        source,
    })
}
fn validate(path: &Path, settings: &Settings) -> Result<(), Error> {
    settings
        .appearance
        .validate()
        .map_err(|e| invalid(path, e))?;
    validate_registrations(&settings.daemon.agents).map_err(|e| invalid(path, e))?;
    settings
        .daemon
        .acp
        .validate(&settings.daemon.agents)
        .map_err(|e| invalid(path, e))?;
    settings.frontend.validate().map_err(|e| invalid(path, e))
}
fn reject_links(path: &Path) -> Result<(), Error> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::Symlink {
                    path: ancestor.into(),
                });
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(ancestor, e)),
        }
    }
    Ok(())
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Ok(_) => Err(io_error(path, e)),
            Err(missing) if missing.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(path, error)),
        },
        Err(e) => Err(io_error(path, e)),
    }
}
