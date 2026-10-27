//! Separately owned frontend and daemon settings with conflict-aware persistence.
pub mod appearance;
pub mod connections;
mod store;
pub use appearance::Appearance;
pub use bas_client::config::{
    AcpLifecycle, AgentSpec, Daemon, MAX_ACP_POOL_RESERVE, agent, paths, registration,
    validate_agents,
};
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
#[cfg(test)]
mod tests;
