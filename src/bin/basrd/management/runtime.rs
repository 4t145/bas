use super::{Error, Progress, artifact, registry};
use anyhow::{Context, Result};
use basr::config::agent::Command;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

mod process;

const MIN_NODE_MAJOR: u64 = 22;
const NODE_INDEX: &str = "https://nodejs.org/dist/index.json";
const PI_PACKAGE: &str = "@earendil-works/pi-coding-agent";
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_VERSION_BYTES: u64 = 1024;
const MAX_ERROR_BYTES: u64 = 8192;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Runtime {
    pub node: PathBuf,
    pub npm_cli: PathBuf,
    pub version: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    version: String,
    lts: serde_json::Value,
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Package {
    version: String,
    #[serde(default)]
    bin: serde_json::Value,
}

pub struct Installed {
    pub command: Command,
    pub env: BTreeMap<String, String>,
    pub runtime: Runtime,
    pub dependencies: BTreeMap<String, String>,
}

/// # Errors
/// Reports missing compatible runtimes, failed verified Node downloads or cancellation.
pub async fn ensure(client: &reqwest::Client, root: &Path, progress: &Progress) -> Result<Runtime> {
    progress.check()?;
    let system = system_runtime(progress).await;
    progress.check()?;
    if let Some(runtime) = system {
        return Ok(runtime);
    }
    let runtime_file = root.join("node-runtime.json");
    artifact::reject_file_link(&runtime_file)?;
    if let Ok(bytes) = fs::read(&runtime_file)
        && let Ok(runtime) = serde_json::from_slice::<Runtime>(&bytes)
        && runtime.node.starts_with(root)
        && runtime.npm_cli.starts_with(root)
        && runtime.node.is_file()
        && runtime.npm_cli.is_file()
    {
        return Ok(runtime);
    }
    progress
        .send("runtime", "正在安装私有 Node.js 运行时")
        .await?;
    let releases: Vec<Release> = get_json(client, NODE_INDEX, progress).await?;
    let node_platform = node_platform()?;
    let release = releases
        .into_iter()
        .find(|release| {
            release.lts.is_string()
                && release
                    .version
                    .strip_prefix('v')
                    .and_then(|v| semver::Version::parse(v).ok())
                    .is_some_and(|v| v.major >= MIN_NODE_MAJOR)
                && release.files.contains(&if cfg!(windows) {
                    format!("{node_platform}-zip")
                } else {
                    node_platform.clone()
                })
        })
        .context("official Node index has no compatible LTS runtime")?;
    registry::validate_component(&release.version)?;
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    let filename = format!("node-{}-{node_platform}.{extension}", release.version);
    let base = format!("https://nodejs.org/dist/{}/", release.version);
    let checksums = get_text(client, &format!("{base}SHASUMS256.txt"), progress).await?;
    let digest = checksums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let digest = parts.next()?;
            (parts.next()? == filename).then_some(digest.to_owned())
        })
        .context("Node archive missing from official checksum list")?;
    let runtimes = root.join("runtimes");
    artifact::safe_directory(&runtimes)?;
    let staging = tempfile::Builder::new()
        .prefix(".node-")
        .tempdir_in(&runtimes)?;
    let archive = staging.path().join("download");
    let url = format!("{base}{filename}");
    artifact::download(client, &url, &archive, Some(&digest), progress).await?;
    let extracted = tokio::task::spawn_blocking(move || -> Result<tempfile::TempDir> {
        artifact::extract(&archive, &url, staging.path(), "node", true)?;
        fs::remove_file(archive)?;
        Ok(staging)
    })
    .await??;
    progress.check()?;
    let dirname = format!("node-{}-{node_platform}", release.version);
    let relative_node = PathBuf::from(&dirname).join(if cfg!(windows) {
        "node.exe"
    } else {
        "bin/node"
    });
    let relative_npm = PathBuf::from(&dirname).join(if cfg!(windows) {
        "node_modules/npm/bin/npm-cli.js"
    } else {
        "lib/node_modules/npm/bin/npm-cli.js"
    });
    artifact::executable(&extracted.path().join(&relative_node))?;
    let destination = runtimes.join(format!("{}-{node_platform}", release.version));
    artifact::reject_file_link(&destination)?;
    if !destination.exists() {
        fs::rename(extracted.path(), &destination)?;
    }
    let runtime = Runtime {
        node: destination.join(relative_node),
        npm_cli: destination.join(relative_npm),
        version: release.version,
    };
    if !runtime.npm_cli.is_file() {
        return Err(Error::MissingExecutable(runtime.npm_cli.clone()).into());
    }
    artifact::atomic_json(&runtime_file, &runtime)?;
    Ok(runtime)
}

async fn system_runtime(progress: &Progress) -> Option<Runtime> {
    let node = fs::canonicalize(which::which("node").ok()?).ok()?;
    let npm = which::which("npm").ok()?;
    let output = tempfile::NamedTempFile::new().ok()?;
    let mut command = tokio::process::Command::new(&node);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.reopen().ok()?))
        .stderr(Stdio::null());
    let version = process::run(command, async {
        tokio::select! {
            () = tokio::time::sleep(VERSION_TIMEOUT) => {}
            () = progress.closed() => {}
        }
    })
    .await
    .ok()?;
    if !matches!(version, process::Outcome::Exited(status) if status.success()) {
        return None;
    }
    let mut bytes = Vec::new();
    output
        .reopen()
        .ok()?
        .take(MAX_VERSION_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_VERSION_BYTES {
        return None;
    }
    let version = String::from_utf8(bytes).ok()?.trim().to_owned();
    if semver::Version::parse(version.strip_prefix('v')?)
        .ok()?
        .major
        < MIN_NODE_MAJOR
    {
        return None;
    }
    let npm_cli = fs::canonicalize(npm).ok()?;
    // npm is a shell shim on Windows; use its JS entrypoint when the installation exposes one.
    let npm_cli = if npm_cli.extension().is_some_and(|ext| ext == "cmd") {
        npm_cli.parent()?.join("node_modules/npm/bin/npm-cli.js")
    } else {
        npm_cli
    };
    if !npm_cli.is_file() {
        return None;
    }
    Some(Runtime {
        node,
        npm_cli,
        version,
    })
}

/// # Errors
/// Rejects unsupported host architectures for official Node runtime archives.
fn node_platform() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return Err(Error::UnsupportedRuntime.into()),
    };
    Ok(format!("{os}-{arch}"))
}

/// # Errors
/// Reports failed pinned npm installs, missing package bins or cancellation.
pub async fn install(
    client: &reqwest::Client,
    root: &Path,
    staging: &Path,
    destination: &Path,
    manifest: &registry::Manifest,
    npm: &registry::Npm,
    progress: &Progress,
) -> Result<Installed> {
    let runtime = ensure(client, root, progress).await?;
    let name = registry::pinned_package(&npm.package, &manifest.version)?;
    let mut dependencies = BTreeMap::from([(name.to_owned(), manifest.version.clone())]);
    if manifest.id == "pi-acp" {
        let metadata: Package = get_json(
            client,
            "https://registry.npmjs.org/@earendil-works%2fpi-coding-agent/latest",
            progress,
        )
        .await?;
        registry::pinned_package(
            &format!("{PI_PACKAGE}@{}", metadata.version),
            &metadata.version,
        )?;
        dependencies.insert(PI_PACKAGE.into(), metadata.version);
    }
    artifact::atomic_json(
        &staging.join("package.json"),
        &serde_json::json!({"private":true,"name":"basr-managed-agent","version":"1.0.0","dependencies":dependencies}),
    )?;
    progress
        .send("installing", "正在安装固定版本的 npm 后端及依赖")
        .await?;
    let mut command = tokio::process::Command::new(&runtime.node);
    command
        .arg(&runtime.npm_cli)
        .args([
            "install",
            "--global=false",
            "--no-audit",
            "--no-fund",
            "--no-update-notifier",
            "--engine-strict",
            "--save-exact",
            "--package-lock=true",
        ])
        .arg("--prefix")
        .arg(staging)
        .current_dir(staging)
        .envs(&npm.env)
        .env(
            "PATH",
            launch_path(&runtime, staging, npm.env.get("PATH").map(String::as_str))?,
        )
        .env("npm_config_cache", root.join("npm-cache"))
        .env("npm_config_update_notifier", "false")
        .stdin(Stdio::null());
    run(command, progress).await?;
    artifact::adapt_tree(&staging.join("node_modules"))?;
    let package_root = staging.join("node_modules").join(name);
    let metadata: Package = serde_json::from_slice(&fs::read(package_root.join("package.json"))?)?;
    if metadata.version != manifest.version {
        return Err(Error::PackageVersion.into());
    }
    let entrypoint = package_entrypoint(name, &metadata.bin)?;
    let program = package_root.join(&entrypoint);
    let canonical = fs::canonicalize(&program)?;
    if !canonical.starts_with(staging) || !canonical.is_file() {
        return Err(Error::MissingExecutable(program).into());
    }
    let relative = canonical.strip_prefix(staging)?;
    let final_program = destination.join(relative);
    let mut args = npm.args.clone();
    let command = if canonical
        .extension()
        .is_some_and(|ext| ext == "js" || ext == "cjs" || ext == "mjs")
    {
        args.insert(0, final_program.to_string_lossy().into_owned());
        Command::new(&runtime.node, args)
    } else {
        artifact::executable(&canonical)?;
        Command::new(final_program, args)
    };
    let mut env = npm.env.clone();
    env.insert(
        "PATH".into(),
        launch_path(
            &runtime,
            destination,
            npm.env.get("PATH").map(String::as_str),
        )?
        .to_string_lossy()
        .into_owned(),
    );
    Ok(Installed {
        command,
        env,
        runtime,
        dependencies,
    })
}

fn package_entrypoint(name: &str, bin: &serde_json::Value) -> Result<PathBuf> {
    let path = match bin {
        serde_json::Value::String(path) => Some(path.as_str()),
        serde_json::Value::Object(bins) => {
            let short_name = name.rsplit('/').next().context("npm package has no name")?;
            bins.get(short_name)
                .and_then(|value| value.as_str())
                .or_else(|| {
                    (bins.len() == 1)
                        .then(|| bins.values().next().and_then(|value| value.as_str()))
                        .flatten()
                })
        }
        _ => None,
    }
    .context("npm package has no unambiguous executable")?;
    Ok(registry::safe_relative(path)?)
}

fn launch_path(
    runtime: &Runtime,
    directory: &Path,
    override_path: Option<&str>,
) -> Result<std::ffi::OsString> {
    let mut paths = vec![
        directory.join("node_modules/.bin"),
        runtime
            .node
            .parent()
            .context("Node has no parent")?
            .to_path_buf(),
    ];
    let inherited_path = override_path
        .map(std::ffi::OsString::from)
        .unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default());
    paths.extend(std::env::split_paths(&inherited_path));
    Ok(std::env::join_paths(paths)?)
}

async fn run(mut command: tokio::process::Command, progress: &Progress) -> Result<()> {
    progress.check()?;
    let log = tempfile::NamedTempFile::new()?;
    command
        .stdout(Stdio::from(log.reopen()?))
        .stderr(Stdio::from(log.reopen()?));
    let status = match process::run(command, progress.closed()).await? {
        process::Outcome::Exited(status) => status,
        process::Outcome::Cancelled => return Err(Error::Cancelled.into()),
    };
    if !status.success() {
        let mut log_file = log.reopen()?;
        let length = log_file.metadata()?.len();
        log_file.seek(std::io::SeekFrom::Start(
            length.saturating_sub(MAX_ERROR_BYTES),
        ))?;
        let mut bytes = Vec::new();
        log_file.take(MAX_ERROR_BYTES).read_to_end(&mut bytes)?;
        let detail = String::from_utf8_lossy(&bytes);
        anyhow::bail!("npm installation failed ({status}): {detail}");
    }
    Ok(())
}

/// # Errors
/// Reports registry network/JSON errors and cancellation.
pub async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    progress: &Progress,
) -> Result<T> {
    let text = get_text(client, url, progress).await?;
    Ok(serde_json::from_str(&text)?)
}

/// # Errors
/// Reports download errors, oversized metadata and cancellation.
async fn get_text(client: &reqwest::Client, url: &str, progress: &Progress) -> Result<String> {
    let response = tokio::select! { response = client.get(registry::download_url(url)?).timeout(std::time::Duration::from_secs(30)).send() => response?.error_for_status()?, _ = progress.closed() => return Err(Error::Cancelled.into()) };
    registry::download_url(response.url().as_str())?;
    const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    loop {
        let chunk = tokio::select! { chunk = stream.next() => chunk, _ = progress.closed() => return Err(Error::Cancelled.into()) };
        let Some(chunk) = chunk else {
            break;
        };
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > MAX_METADATA_BYTES {
            return Err(Error::ArchiveLimit.into());
        }
    }
    Ok(String::from_utf8(bytes)?)
}

use futures::StreamExt;

#[cfg(test)]
mod tests;
