//! Configuration payloads exchanged with BAS; this module never owns settings files.

mod acp;
pub mod agent;
pub mod paths;
pub mod registration;

pub use acp::{AcpLifecycle, MAX_ACP_POOL_RESERVE};
pub use agent::{AgentSpec, validate_agents};
pub use registration::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    pub agents: Vec<AgentRegistration>,
    pub acp: AcpLifecycle,
}

impl Daemon {
    /// Validate an RPC configuration without reading local installations.
    ///
    /// # Errors
    /// Reports invalid registrations or ACP policy references.
    pub fn validate(&self) -> Result<(), RegistrationError> {
        validate_registrations(&self.agents)?;
        self.acp.validate(&self.agents)?;
        Ok(())
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self {
            agents: vec![AgentSpec::embedded().into()],
            acp: AcpLifecycle::default(),
        }
    }
}
