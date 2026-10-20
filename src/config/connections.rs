use crate::service::Target;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[cfg(test)]
mod tests;

/// Saved frontend endpoints, independent of transport support on this platform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Frontend {
    pub connections: Vec<ConnectionProfile>,
    pub selected: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(with = "target")]
    pub target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<crate::service::tls::ClientTls>,
}

impl ConnectionProfile {
    pub fn options(&self) -> crate::service::tls::ConnectionOptions {
        crate::service::tls::ConnectionOptions {
            target: self.target.clone(),
            tls: self.tls.clone(),
        }
    }
}

impl Default for Frontend {
    fn default() -> Self {
        Self {
            connections: vec![ConnectionProfile {
                id: "local".into(),
                name: "Local".into(),
                note: None,
                target: Target::Local,
                tls: None,
            }],
            selected: "local".into(),
        }
    }
}

impl Frontend {
    /// # Errors
    /// Rejects empty lists, blank identities, duplicate IDs and missing selection.
    pub fn validate(&self) -> Result<(), String> {
        if self.connections.is_empty() {
            return Err("frontend.connections must not be empty".into());
        }
        let mut ids = HashSet::new();
        for connection in &self.connections {
            connection
                .options()
                .validate()
                .map_err(|error| error.to_string())?;
            if connection.id.trim().is_empty() || connection.name.trim().is_empty() {
                return Err("frontend connection id and name must not be blank".into());
            }
            if !ids.insert(&connection.id) {
                return Err(format!(
                    "duplicate frontend connection id: {}",
                    connection.id
                ));
            }
        }
        if self.selected_connection().is_none() {
            return Err("frontend.selected must reference an existing connection".into());
        }
        Ok(())
    }

    pub fn selected_connection(&self) -> Option<&ConnectionProfile> {
        self.connections
            .iter()
            .find(|connection| connection.id == self.selected)
    }
}

mod target {
    use crate::service::Target;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(target: &Target, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(target)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Target, D::Error> {
        Target::from_str(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}
