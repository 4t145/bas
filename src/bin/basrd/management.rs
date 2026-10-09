//! The daemon owns ACP downloads, runtime provisioning and settings registration.

mod artifact;
mod authentication;
mod registry;
mod runtime;
pub use authentication::invocation as authentication_invocation;

use anyhow::{Context, Result};
use basr::{
    config::{
        AgentSpec, Store,
        agent::{Command, Kind},
    },
    management::{Catalog, Entry, Source, Status},
    service::pb::AgentOperation,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf, time::Duration};
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("installation cancelled")]
    Cancelled,
    #[error(
        "this daemon uses command-line agent overrides; managed installation requires a settings store"
    )]
    NoStore,
    #[error("agent {0} was not found in the ACP registry")]
    UnknownAgent(String),
    #[error(
        "agent {0} is configured manually or its managed settings were edited; use another ID or remove the custom configuration first"
    )]
    RegistrationConflict(String),
    #[error("agent {0} is not a managed installation")]
    NotManaged(String),
    #[error("downloaded archive checksum does not match the registry")]
    Checksum,
    #[error("download or archive exceeds the supported size/file limit")]
    ArchiveLimit,
    #[error("unsupported archive format")]
    UnsupportedArchive,
    #[error("archive contains a link or special file: {0}")]
    ArchiveLink(String),
    #[error("managed storage cannot contain a symbolic link: {0}")]
    StorageLink(PathBuf),
    #[error("managed installation has no regular executable at {0}")]
    MissingExecutable(PathBuf),
    #[error("official Node runtime is unavailable for this architecture")]
    UnsupportedRuntime,
    #[error("installed npm package version differs from the registry")]
    PackageVersion,
    #[error("managed installation metadata has an unsupported schema version")]
    RecordsVersion,
    #[error("another daemon is installing or removing agents")]
    Busy,
}

#[derive(Clone)]
struct Progress(Option<mpsc::Sender<AgentOperation>>);
impl Progress {
    async fn send(&self, phase: &str, message: &str) -> Result<()> {
        if let Some(sender) = &self.0 {
            sender
                .send(AgentOperation {
                    phase: phase.into(),
                    message: message.into(),
                    spec_json: Vec::new(),
                    complete: false,
                })
                .await
                .map_err(|_| Error::Cancelled)?;
        }
        Ok(())
    }
    async fn closed(&self) {
        if let Some(sender) = &self.0 {
            sender.closed().await;
        } else {
            std::future::pending::<()>().await;
        }
    }
    fn check(&self) -> Result<(), Error> {
        if self.0.as_ref().is_some_and(mpsc::Sender::is_closed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Record {
    manifest: registry::Manifest,
    directory: PathBuf,
    spec: AgentSpec,
    runtime: Option<runtime::Runtime>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    launch: Option<authentication::LaunchBoundary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Records {
    schema_version: u32,
    agents: BTreeMap<String, Record>,
}
impl Default for Records {
    fn default() -> Self {
        Self {
            schema_version: 1,
            agents: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Cache {
    url: String,
    registry: registry::Registry,
}

#[derive(Default)]
struct RegistryMemory {
    registry: Option<registry::Registry>,
    warning: Option<String>,
}

pub struct Manager {
    store: Option<Store>,
    root: PathBuf,
    url: String,
    client: reqwest::Client,
    registry: Mutex<RegistryMemory>,
    mutation: Mutex<()>,
}

impl Manager {
    /// # Errors
    /// Rejects unsafe data directories/registry URLs and HTTP client initialization failures.
    pub fn new(store: Option<Store>) -> Result<Self> {
        let root = match std::env::var_os("BASRD_ACP_DATA_DIR") {
            Some(path) => PathBuf::from(path),
            None => dirs::data_local_dir()
                .context("user data directory is unavailable")?
                .join("basr/acp"),
        };
        if !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            anyhow::bail!("BASRD_ACP_DATA_DIR must be absolute without parent traversal");
        }
        artifact::reject_directory_links(&root)?;
        let url = std::env::var("BASRD_ACP_REGISTRY_URL")
            .unwrap_or_else(|_| registry::REGISTRY_URL.into());
        registry::download_url(&url)?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("basr/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("too many redirects")
                } else if registry::download_url(attempt.url().as_str()).is_err() {
                    attempt.error("unsafe download redirect")
                } else {
                    attempt.follow()
                }
            }))
            .build()?;
        Ok(Self {
            store,
            root,
            url,
            client,
            registry: Mutex::new(RegistryMemory::default()),
            mutation: Mutex::new(()),
        })
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    /// # Errors
    /// Reports invalid installation metadata; registry outages are returned as catalog warnings.
    pub async fn catalog(&self, specs: Vec<AgentSpec>, refresh: bool) -> Result<Catalog> {
        let loaded = self.load_registry(refresh, &Progress(None)).await;
        let (registry, warning) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => (None, Some(format!("ACP 目录暂不可用：{error}"))),
        };
        let records = self.records()?;
        let mut configured: BTreeMap<_, _> = specs
            .into_iter()
            .map(|spec| (spec.id.as_str().to_owned(), spec))
            .collect();
        let mut entries = Vec::new();
        if let Some(registry) = registry {
            for manifest in registry.agents {
                let spec = configured.remove(&manifest.id);
                let record = records.agents.get(&manifest.id);
                let managed = record.is_some_and(|record| spec.as_ref() == Some(&record.spec));
                let custom = spec.is_some() && !managed;
                let (status, message) = if let Some(spec) = &spec {
                    let ready = if managed {
                        record.is_some_and(record_available)
                    } else {
                        available(spec)
                    };
                    if ready {
                        (
                            Status::Installed,
                            if custom {
                                "自定义配置".into()
                            } else {
                                "安装完成".into()
                            },
                        )
                    } else {
                        (
                            Status::Missing,
                            "启动程序不存在；请重新安装或检查高级配置".into(),
                        )
                    }
                } else {
                    match manifest.select() {
                        Ok(_) => (Status::NotInstalled, String::new()),
                        Err(error) => (Status::Unsupported, error.to_string()),
                    }
                };
                entries.push(Entry {
                    id: manifest.id.clone(),
                    name: if custom {
                        manifest.id.clone()
                    } else {
                        manifest.name
                    },
                    description: manifest.description,
                    version: manifest.version,
                    repository: manifest.repository,
                    license: manifest.license,
                    license_url: manifest.license_url,
                    icon: manifest.icon,
                    installed_version: managed
                        .then(|| record.map(|record| record.manifest.version.clone()))
                        .flatten(),
                    source: if custom {
                        Source::Custom
                    } else {
                        Source::Registry
                    },
                    status,
                    message,
                    spec,
                });
            }
        }
        for (id, spec) in configured {
            let record = records.agents.get(&id).filter(|record| record.spec == spec);
            let source = if spec.kind == Kind::Embedded {
                Source::Embedded
            } else if record.is_some() {
                Source::Registry
            } else {
                Source::Custom
            };
            entries.push(Entry {
                name: record.map_or_else(|| id.clone(), |record| record.manifest.name.clone()),
                id,
                description: record
                    .map_or_else(String::new, |record| record.manifest.description.clone()),
                version: record.map_or_else(String::new, |record| record.manifest.version.clone()),
                repository: record
                    .map_or_else(String::new, |record| record.manifest.repository.clone()),
                license: record.map_or_else(String::new, |record| record.manifest.license.clone()),
                license_url: record
                    .map_or_else(String::new, |record| record.manifest.license_url.clone()),
                icon: record.map_or_else(String::new, |record| record.manifest.icon.clone()),
                installed_version: record.map(|record| record.manifest.version.clone()),
                source,
                status: if record.map_or_else(|| available(&spec), record_available) {
                    Status::Installed
                } else {
                    Status::Missing
                },
                message: if record.map_or_else(|| available(&spec), record_available) {
                    String::new()
                } else {
                    "启动程序不存在；检查高级配置或重新安装".into()
                },
                spec: Some(spec),
            });
        }
        entries.sort_by_key(|entry| entry.name.to_lowercase());
        Ok(Catalog {
            entries,
            warning,
            settings_path: self.store.as_ref().map(|store| store.path().to_path_buf()),
            can_install: self.store.is_some(),
        })
    }

    /// # Errors
    /// Reports unknown/unsupported agents, cancellation, download/runtime failures and settings conflicts.
    pub async fn install(
        &self,
        id: &str,
        progress: mpsc::Sender<AgentOperation>,
    ) -> Result<AgentSpec> {
        registry::validate_component(id)?;
        let progress = Progress(Some(progress));
        let _guard = tokio::select! { guard = self.mutation.lock() => guard, _ = progress.closed() => return Err(Error::Cancelled.into()) };
        let _disk_lock = self.mutation_lock()?;
        let store = self.store.as_ref().ok_or(Error::NoStore)?;
        // Preserve this snapshot throughout downloads so external edits are never overwritten.
        let mut document = store.load()?;
        let old_agents = document.settings.daemon.agents.clone();
        let mut records = self.records()?;
        check_registration(id, &old_agents, &records)?;
        progress
            .send("resolving", "正在读取 ACP 后端安装信息")
            .await?;
        let (registry, _) = self.load_registry(false, &progress).await?;
        let registry = registry.context("ACP registry is unavailable")?;
        let manifest = registry
            .agents
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| Error::UnknownAgent(id.into()))?;
        let selected = manifest.select()?;
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&manifest)?));
        let versions = self.root.join("agents").join(id);
        artifact::safe_directory(&versions)?;
        // Different runtime/dependency resolutions must never reuse a previous version's contents.
        let destination = versions.join(format!(
            "{}-{}-{}",
            manifest.version,
            &fingerprint[..12],
            uuid::Uuid::new_v4().simple()
        ));
        artifact::reject_file_link(&destination)?;
        let mut staging = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(&versions)?;
        let (spec, runtime, dependencies) = match selected {
            registry::Selected::Binary(binary) => {
                progress.send("downloading", "正在下载后端程序").await?;
                let archive = staging.path().join("download");
                artifact::download(
                    &self.client,
                    &binary.archive,
                    &archive,
                    binary.sha256.as_deref(),
                    &progress,
                )
                .await?;
                progress
                    .send("extracting", "正在校验并解压后端程序")
                    .await?;
                let url = binary.archive.clone();
                let executable = binary.cmd.clone();
                staging = tokio::task::spawn_blocking(move || -> Result<tempfile::TempDir> {
                    artifact::extract(&archive, &url, staging.path(), &executable, false)?;
                    fs::remove_file(archive)?;
                    artifact::executable(
                        &staging.path().join(registry::safe_relative(&executable)?),
                    )?;
                    Ok(staging)
                })
                .await??;
                let mut spec = AgentSpec::acp(
                    id,
                    Command::new(
                        destination.join(registry::safe_relative(&binary.cmd)?),
                        binary.args.clone(),
                    ),
                );
                spec.env = binary.env.clone();
                (spec, None, BTreeMap::new())
            }
            registry::Selected::Npm(npm) => {
                let installed = runtime::install(
                    &self.client,
                    &self.root,
                    staging.path(),
                    &destination,
                    &manifest,
                    npm,
                    &progress,
                )
                .await?;
                let mut spec = AgentSpec::acp(id, installed.command);
                spec.env = installed.env;
                (spec, Some(installed.runtime), installed.dependencies)
            }
        };
        artifact::atomic_json(&staging.path().join("basr-installation.json"), &manifest)?;
        progress.send("registering", "正在保存后端启动配置").await?;
        progress.check()?;
        fs::rename(staging.path(), &destination)?;
        let mut prepared_directory = PreparedDirectory(Some(destination.clone()));
        let mut record = Record {
            manifest,
            directory: destination,
            spec: spec.clone(),
            runtime,
            dependencies,
            launch: None,
        };
        record.launch = authentication::LaunchBoundary::from_record(&record);
        records.agents.insert(id.into(), record);
        let mut agents = old_agents.clone();
        if let Some(existing) = agents.iter_mut().find(|agent| agent.id.as_str() == id) {
            *existing = spec.clone();
        } else {
            agents.push(spec.clone());
        }
        self.commit(store, &mut document, agents, old_agents, &records)?;
        prepared_directory.0 = None;
        Ok(spec)
    }

    /// # Errors
    /// Rejects custom/edited registrations, failed settings saves and record writes.
    pub async fn remove(&self, id: &str) -> Result<Vec<AgentSpec>> {
        registry::validate_component(id)?;
        let _guard = self.mutation.lock().await;
        let _disk_lock = self.mutation_lock()?;
        let store = self.store.as_ref().ok_or(Error::NoStore)?;
        let mut document = store.load()?;
        let old_agents = document.settings.daemon.agents.clone();
        let mut records = self.records()?;
        if !records.agents.contains_key(id) {
            return Err(Error::NotManaged(id.into()).into());
        }
        check_registration(id, &old_agents, &records)?;
        let mut agents: Vec<_> = old_agents
            .iter()
            .filter(|agent| agent.id.as_str() != id)
            .cloned()
            .collect();
        if agents.is_empty() {
            agents.push(AgentSpec::embedded());
        }
        records.agents.remove(id);
        self.commit(store, &mut document, agents.clone(), old_agents, &records)?;
        Ok(agents)
    }

    fn records(&self) -> Result<Records> {
        let path = self.root.join("installations.json");
        artifact::reject_file_link(&path)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Records::default());
            }
            Err(error) => return Err(error.into()),
        };
        let records: Records =
            serde_json::from_slice(&bytes).context("invalid managed installation metadata")?;
        if records.schema_version != 1 {
            return Err(Error::RecordsVersion.into());
        }
        for (id, record) in &records.agents {
            registry::validate_component(id)?;
            if id != &record.manifest.id
                || record.spec.id.as_str() != id
                || !record
                    .directory
                    .starts_with(self.root.join("agents").join(id))
                || record
                    .directory
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                anyhow::bail!("managed installation metadata has an invalid identity or directory");
            }
        }
        Ok(records)
    }

    fn mutation_lock(&self) -> Result<DiskLock> {
        artifact::safe_directory(&self.root)?;
        let path = self.root.join("operation.lock");
        artifact::reject_file_link(&path)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(path)?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => anyhow::Error::from(Error::Busy),
            fs::TryLockError::Error(error) => error.into(),
        })?;
        Ok(DiskLock(file))
    }

    fn commit(
        &self,
        store: &Store,
        document: &mut basr::config::Document,
        agents: Vec<AgentSpec>,
        old_agents: Vec<AgentSpec>,
        records: &Records,
    ) -> Result<()> {
        let path = self.root.join("installations.json");
        artifact::reject_file_link(&path)?;
        let mut prepared = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer_pretty(&mut prepared, records)?;
        prepared.write_all(b"\n")?;
        prepared.as_file().sync_all()?;
        store.save_agents(document, agents)?;
        if let Err(error) = prepared.persist(path) {
            store
                .save_agents(document, old_agents)
                .context("failed to restore settings after installation metadata write failed")?;
            return Err(error.into());
        }
        Ok(())
    }

    async fn load_registry(
        &self,
        refresh: bool,
        progress: &Progress,
    ) -> Result<(Option<registry::Registry>, Option<String>)> {
        let mut memory = self.registry.lock().await;
        let cache_path = self.root.join("registry.json");
        if memory.registry.is_none() {
            artifact::reject_file_link(&cache_path)?;
            if let Ok(bytes) = fs::read(&cache_path)
                && let Ok(cache) = serde_json::from_slice::<Cache>(&bytes)
                && cache.url == self.url
                && cache.registry.validate().is_ok()
            {
                memory.registry = Some(cache.registry);
            }
        }
        if !refresh && memory.registry.is_some() {
            return Ok((memory.registry.clone(), memory.warning.clone()));
        }
        let downloaded: Result<registry::Registry> = async {
            let registry: registry::Registry =
                runtime::get_json(&self.client, &self.url, progress).await?;
            registry.validate()?;
            Ok(registry)
        }
        .await;
        match downloaded {
            Ok(registry) => {
                let warning = artifact::atomic_json(
                    &cache_path,
                    &Cache {
                        url: self.url.clone(),
                        registry: registry.clone(),
                    },
                )
                .err()
                .map(|error| format!("ACP 目录缓存保存失败：{error}"));
                memory.registry = Some(registry.clone());
                memory.warning = warning.clone();
                Ok((Some(registry), warning))
            }
            Err(error) if memory.registry.is_some() => {
                progress.check()?;
                memory.warning = Some(format!("ACP 目录刷新失败，正在使用缓存：{error}"));
                Ok((memory.registry.clone(), memory.warning.clone()))
            }
            Err(error) => Err(error),
        }
    }
}

struct DiskLock(fs::File);
impl Drop for DiskLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

struct PreparedDirectory(Option<PathBuf>);
impl Drop for PreparedDirectory {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn check_registration(id: &str, agents: &[AgentSpec], records: &Records) -> Result<(), Error> {
    if let Some(existing) = agents.iter().find(|agent| agent.id.as_str() == id)
        && records
            .agents
            .get(id)
            .is_none_or(|record| record.spec != *existing)
    {
        return Err(Error::RegistrationConflict(id.into()));
    }
    Ok(())
}

fn available(spec: &AgentSpec) -> bool {
    if spec.kind == Kind::Embedded {
        return true;
    }
    crate::agent::acp::resolve_program(spec).is_some()
}

fn record_available(record: &Record) -> bool {
    if !available(&record.spec) || !record.directory.is_dir() {
        return false;
    }
    match record.manifest.select() {
        Ok(registry::Selected::Binary(binary)) => registry::safe_relative(&binary.cmd)
            .is_ok_and(|path| record.directory.join(path).is_file()),
        Ok(registry::Selected::Npm(_)) => {
            let dependencies_ready = record.dependencies.iter().all(|(name, version)| {
                fs::read(
                    record
                        .directory
                        .join("node_modules")
                        .join(name)
                        .join("package.json"),
                )
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|package| {
                    package.get("version").and_then(|version| version.as_str())
                        == Some(version.as_str())
                })
            });
            let command_ready = record.spec.command().is_some_and(|command| {
                command.program.starts_with(&record.directory)
                    || command.args.first().is_some_and(|arg| {
                        let path = PathBuf::from(arg);
                        path.starts_with(&record.directory) && path.is_file()
                    })
            });
            dependencies_ready && command_ready
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;
