//! Separately owned frontend and daemon settings with conflict-aware persistence.
mod acp;
pub mod agent;
pub mod appearance;
pub mod connections;
pub mod paths;
pub mod registration;
mod store;
pub use acp::{AcpLifecycle, MAX_ACP_POOL_RESERVE};
pub use agent::{AgentSpec, validate_agents};
pub use appearance::Appearance;
pub use connections::{ConnectionProfile, Frontend};
pub use registration::*;
use serde::{Deserialize, Serialize};
pub use store::{
    DaemonDocument, DaemonStore, Document, Error, FrontendDocument, FrontendStore, Store,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FrontendSettings {
    pub appearance: Appearance,
    pub frontend: Frontend,
}

/// Legacy unified representation for imports and compatibility tools.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub appearance: Appearance,
    pub daemon: Daemon,
    pub frontend: Frontend,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    pub agents: Vec<AgentRegistration>,
    pub acp: AcpLifecycle,
}
impl Default for Daemon {
    fn default() -> Self {
        Self {
            agents: vec![AgentSpec::embedded().into()],
            acp: AcpLifecycle::default(),
        }
    }
}
#[cfg(test)]
mod tests;
