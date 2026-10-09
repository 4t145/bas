//! Agent discovery and installation metadata shared by daemon and clients.

use crate::config::AgentSpec;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub warning: Option<String>,
    pub settings_path: Option<PathBuf>,
    pub can_install: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repository: String,
    pub license: String,
    pub license_url: String,
    pub icon: String,
    pub installed_version: Option<String>,
    pub source: Source,
    pub status: Status,
    pub message: String,
    pub spec: Option<AgentSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Registry,
    Custom,
    Embedded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NotInstalled,
    Installed,
    Missing,
    Unsupported,
}
