use super::*;

#[cfg(test)]
mod tests;

impl Store {
    /// Frontend preferences never decode or validate backend-owned fields.
    pub(super) fn frontend(path: Option<PathBuf>) -> Self {
        let mut store = Self::new(path);
        store.scope = Scope::Frontend;
        store
    }

    /// Backend configuration defaults to daemon.json, with read-only legacy import.
    pub(super) fn daemon(path: Option<PathBuf>) -> Self {
        let mut store = match path {
            Some(path) => Self::at(path),
            None => Self {
                path: super::super::paths::config_root()
                    .unwrap_or_else(|| PathBuf::from(".config"))
                    .join("bas/daemon.json"),
                import_legacy: true,
                scope: Scope::Daemon,
            },
        };
        store.scope = Scope::Daemon;
        store
    }

    fn decode_scoped(&self, path: &Path, raw: serde_json::Value) -> Result<Settings, Error> {
        let mut settings = Settings::default();
        match self.scope {
            Scope::Frontend => {
                let mut owned = raw;
                if let Some(object) = owned.as_object_mut() {
                    object.remove("daemon");
                }
                settings = serde_json::from_value(owned).map_err(|source| Error::Json {
                    path: path.into(),
                    source,
                })?;
            }
            Scope::Daemon => {
                let owned = if legacy_envelope(&raw) {
                    raw.get("daemon")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    raw
                };
                settings.daemon = serde_json::from_value(owned).map_err(|source| Error::Json {
                    path: path.into(),
                    source,
                })?;
            }
            Scope::Unified => unreachable!("unified settings use strict full-document decoding"),
        }
        self.validate_scoped(&settings)?;
        Ok(settings)
    }

    pub(super) fn read_scoped(&self) -> Result<Loaded, Error> {
        let current = read_optional(&self.path)?;
        let mut snapshot = Snapshot {
            files: vec![(self.path.clone(), current.clone())],
        };
        let mut settings = Settings::default();
        let mut raw = serde_json::Value::Null;
        if let Some(bytes) = current {
            raw = parse(&self.path, &bytes)?;
            settings = self.decode_scoped(&self.path, raw.clone())?;
        } else if self.import_legacy {
            match self.scope {
                Scope::Frontend => {
                    let path = self.parent().join("appearance.json");
                    let bytes = read_optional(&path)?;
                    if let Some(bytes) = &bytes {
                        settings.appearance = parse(&path, bytes)?;
                    }
                    snapshot.files.push((path, bytes));
                }
                Scope::Daemon => {
                    let path = self.parent().join("settings.json");
                    let bytes = read_optional(&path)?;
                    if let Some(bytes) = &bytes {
                        settings = self.decode_scoped(&path, parse(&path, bytes)?)?;
                    } else {
                        let path = self.parent().join("agents.json");
                        let bytes = read_optional(&path)?;
                        if let Some(bytes) = &bytes {
                            let agents: Vec<AgentSpec> = parse(&path, bytes)?;
                            validate_agents(&agents).map_err(|error| invalid(&path, error))?;
                            settings.daemon.agents = agents.into_iter().map(Into::into).collect();
                        }
                        snapshot.files.push((path, bytes));
                    }
                    snapshot.files.push((path, bytes));
                }
                Scope::Unified => unreachable!("scoped reader"),
            }
        }
        self.validate_scoped(&settings)?;
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
            raw,
        })
    }

    pub(super) fn validate_scoped(&self, settings: &Settings) -> Result<(), Error> {
        match self.scope {
            Scope::Unified => validate(&self.path, settings),
            Scope::Frontend => {
                settings
                    .appearance
                    .validate()
                    .map_err(|e| invalid(&self.path, e))?;
                settings
                    .frontend
                    .validate()
                    .map_err(|e| invalid(&self.path, e))
            }
            Scope::Daemon => {
                validate_registrations(&settings.daemon.agents)
                    .map_err(|e| invalid(&self.path, e))?;
                settings
                    .daemon
                    .acp
                    .validate(&settings.daemon.agents)
                    .map_err(|e| invalid(&self.path, e))
            }
        }
    }

    pub(super) fn encode_scoped(
        &self,
        settings: &Settings,
        mut raw: serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        let serialize = |result: Result<serde_json::Value, serde_json::Error>| {
            result.map_err(|source| Error::Json {
                path: self.path.clone(),
                source,
            })
        };
        match self.scope {
            Scope::Unified => serialize(serde_json::to_value(settings)),
            Scope::Frontend => {
                if raw.is_null() {
                    raw = serde_json::json!({});
                }
                raw["appearance"] = serialize(serde_json::to_value(&settings.appearance))?;
                raw["frontend"] = serialize(serde_json::to_value(&settings.frontend))?;
                Ok(raw)
            }
            Scope::Daemon => {
                let daemon = serialize(serde_json::to_value(&settings.daemon))?;
                if legacy_envelope(&raw) {
                    raw["daemon"] = daemon;
                    Ok(raw)
                } else {
                    Ok(daemon)
                }
            }
        }
    }
}

fn legacy_envelope(raw: &serde_json::Value) -> bool {
    raw.get("agents").is_none()
        && raw.get("acp").is_none()
        && (raw.get("daemon").is_some()
            || raw.get("appearance").is_some()
            || raw.get("frontend").is_some())
}
