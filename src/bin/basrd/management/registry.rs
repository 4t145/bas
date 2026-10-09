use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

pub const REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Registry {
    pub version: String,
    pub agents: Vec<Manifest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub license_url: String,
    #[serde(default)]
    pub icon: String,
    pub distribution: Distribution,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Distribution {
    #[serde(default)]
    pub binary: BTreeMap<String, Binary>,
    pub npx: Option<Npm>,
    pub uvx: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Binary {
    pub archive: String,
    pub cmd: String,
    pub sha256: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Npm {
    pub package: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

pub enum Selected<'a> {
    Binary(&'a Binary),
    Npm(&'a Npm),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid registry: {0}")]
    Invalid(String),
    #[error("unsupported distribution for {0} on this platform")]
    Unsupported(String),
    #[error("unsafe relative path {0}")]
    Path(String),
    #[error("downloads require HTTPS; plain HTTP is permitted only for loopback test fixtures")]
    Url,
}

impl Registry {
    /// # Errors
    /// Rejects incompatible schema versions, duplicate IDs and unsafe entry names.
    pub fn validate(&self) -> Result<(), Error> {
        let version =
            semver::Version::parse(&self.version).map_err(|e| Error::Invalid(e.to_string()))?;
        if version.major != 1 {
            return Err(Error::Invalid(format!(
                "unsupported schema {}",
                self.version
            )));
        }
        let mut ids = std::collections::HashSet::new();
        for manifest in &self.agents {
            validate_component(&manifest.id)?;
            validate_component(&manifest.version)?;
            if !ids.insert(&manifest.id) {
                return Err(Error::Invalid(format!("duplicate id {}", manifest.id)));
            }
        }
        Ok(())
    }
}

impl Manifest {
    /// # Errors
    /// Rejects unsupported platforms, unsafe commands, URLs, environment keys and npm versions.
    pub fn select(&self) -> Result<Selected<'_>, Error> {
        if let Some(binary) = self.distribution.binary.get(&platform()) {
            safe_relative(&binary.cmd)?;
            download_url(&binary.archive)?;
            validate_env(&binary.env)?;
            validate_args(&binary.args)?;
            if let Some(checksum) = &binary.sha256
                && (checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(Error::Invalid("invalid SHA-256 digest".into()));
            }
            return Ok(Selected::Binary(binary));
        }
        if let Some(npm) = &self.distribution.npx {
            pinned_package(&npm.package, &self.version)?;
            validate_env(&npm.env)?;
            validate_args(&npm.args)?;
            return Ok(Selected::Npm(npm));
        }
        Err(Error::Unsupported(self.id.clone()))
    }
}

pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// # Errors
/// Rejects identifiers that can escape their installation directory.
pub fn validate_component(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        || !value.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(Error::Invalid(format!("unsafe identifier {value:?}")));
    }
    Ok(())
}

/// # Errors
/// Rejects traversal, drive prefixes and absolute paths on every host platform.
pub fn safe_relative(value: &str) -> Result<PathBuf, Error> {
    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    if normalized.contains([':', '\0'])
        || normalized.starts_with('/')
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::Path(value.into()));
    }
    let path: PathBuf = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    if path.as_os_str().is_empty() {
        return Err(Error::Path(value.into()));
    }
    Ok(path)
}

/// # Errors
/// Rejects npm tags, ranges, URLs and a package version different from its manifest.
pub fn pinned_package<'a>(value: &'a str, version: &str) -> Result<&'a str, Error> {
    let (name, pinned) = value
        .rsplit_once('@')
        .ok_or_else(|| Error::Invalid("npm package must have an exact version".into()))?;
    if pinned != version || semver::Version::parse(pinned).is_err() {
        return Err(Error::Invalid(
            "npm package must match the manifest's exact version".into(),
        ));
    }
    let valid_segment = |s: &str| {
        !s.is_empty()
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    };
    let valid = if let Some(scoped) = name.strip_prefix('@') {
        scoped
            .split_once('/')
            .is_some_and(|(scope, package)| valid_segment(scope) && valid_segment(package))
    } else {
        valid_segment(name)
    };
    if !valid {
        return Err(Error::Invalid("invalid npm package name".into()));
    }
    Ok(name)
}

/// # Errors
/// Rejects non-HTTPS network URLs except HTTP loopback fixtures, userinfo and fragments.
pub fn download_url(value: &str) -> Result<Url, Error> {
    let url = Url::parse(value).map_err(|_| Error::Url)?;
    let loopback = url
        .host_str()
        .and_then(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .is_some_and(|address| address.is_loopback());
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Url);
    }
    Ok(url)
}

fn validate_env(env: &BTreeMap<String, String>) -> Result<(), Error> {
    if env
        .iter()
        .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
    {
        return Err(Error::Invalid("invalid environment override".into()));
    }
    Ok(())
}

fn validate_args(args: &[String]) -> Result<(), Error> {
    if args.iter().any(|argument| argument.contains('\0')) {
        return Err(Error::Invalid("invalid command argument".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
