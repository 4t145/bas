//! Strict shared settings and conflict-aware, section-scoped persistence.
pub mod agent;
pub mod appearance;
pub mod paths;
mod store;
pub use agent::{AgentSpec, validate_agents};
pub use appearance::Appearance;
use serde::{Deserialize, Serialize};
pub use store::{Document, Error, Store};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub appearance: Appearance,
    pub daemon: Daemon,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    pub agents: Vec<AgentSpec>,
}
impl Default for Daemon {
    fn default() -> Self {
        Self {
            agents: vec![AgentSpec::embedded()],
        }
    }
}
#[cfg(test)]
mod tests;
