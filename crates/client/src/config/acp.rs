use super::{
    AgentRegistration,
    agent::{Kind, ValidationError},
};
use crate::model::AgentId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, num::NonZeroU64};

pub const MAX_ACP_POOL_RESERVE: usize = 32;
pub const DEFAULT_ACP_IDLE_TIMEOUT_SECS: u64 = 30 * 60;
pub const DEFAULT_ACP_POOL_SIZE: usize = 1;

/// Server-owned ACP idle lifetime and prewarmed process reserves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AcpLifecycle {
    /// None disables idle eviction; zero is not a valid duration.
    pub idle_timeout_secs: Option<NonZeroU64>,
    pub default_agent: Option<AgentId>,
    pub pool_size: usize,
    /// Explicit entries, including zero, override the automatic target.
    pub pools: BTreeMap<AgentId, usize>,
}

impl Default for AcpLifecycle {
    fn default() -> Self {
        Self {
            idle_timeout_secs: Some(
                NonZeroU64::new(DEFAULT_ACP_IDLE_TIMEOUT_SECS)
                    .expect("default idle timeout is nonzero"),
            ),
            default_agent: None,
            pool_size: DEFAULT_ACP_POOL_SIZE,
            pools: BTreeMap::new(),
        }
    }
}

impl AcpLifecycle {
    pub fn pool_targets(&self, last_used: Option<&AgentId>) -> BTreeMap<AgentId, usize> {
        let mut targets = self.pools.clone();
        if let Some(id) = self.default_agent.as_ref().or(last_used) {
            targets.entry(id.clone()).or_insert(self.pool_size);
        }
        targets
    }

    /// Validates policy without requiring managed installations to be present.
    ///
    /// # Errors
    /// Rejects empty, missing or non-ACP IDs and reserves exceeding the ceiling.
    pub fn validate(&self, agents: &[AgentRegistration]) -> Result<(), ValidationError> {
        for id in self.default_agent.iter().chain(self.pools.keys()) {
            if id.as_str().trim().is_empty() {
                return Err(ValidationError(
                    "ACP policy agent id must not be empty".into(),
                ));
            }
            let agent = agents
                .iter()
                .find(|agent| agent.id() == id)
                .ok_or_else(|| {
                    ValidationError(format!("ACP policy references unknown agent {id}"))
                })?;
            // Managed registrations are ACP-only by contract. Their installation
            // is resolved at daemon publication, not during unrelated settings reads.
            if agent.as_custom().is_some_and(|spec| spec.kind != Kind::Acp) {
                return Err(ValidationError(format!(
                    "ACP policy agent {id} must be ACP"
                )));
            }
        }
        let explicit = self
            .pools
            .values()
            .try_fold(0usize, |sum, size| sum.checked_add(*size));
        let automatic = if self
            .default_agent
            .as_ref()
            .is_some_and(|id| self.pools.contains_key(id))
        {
            0
        } else {
            self.pool_size
        };
        if self.pool_size > MAX_ACP_POOL_RESERVE
            || explicit
                .and_then(|sum| sum.checked_add(automatic))
                .is_none_or(|total| total > MAX_ACP_POOL_RESERVE)
        {
            return Err(ValidationError(format!(
                "ACP pool reserve must not exceed {MAX_ACP_POOL_RESERVE}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
