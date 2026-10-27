//! Connection-scoped child associations, with ACP patch and state semantics.

use crate::acp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Subagents(BTreeMap<String, Subagent>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subagent {
    pub id: String,
    /// None denotes a direct child of the owning Bas conversation.
    pub parent: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<acp::StateUpdate>,
    pub capabilities: Option<acp::SubagentSessionCapabilities>,
}

#[derive(Debug, thiserror::Error)]
pub enum SubagentError {
    #[error("subagent parent has not been announced")]
    UnknownParent,
    #[error("subagent ownership cannot change")]
    Ownership,
}

impl Subagents {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Subagent> {
        self.0.values()
    }

    /// # Errors
    /// Rejects unannounced parents, self-parenting and ownership changes.
    pub fn apply(
        &mut self,
        parent: Option<&str>,
        update: &acp::SubagentUpdate,
    ) -> Result<(), SubagentError> {
        let id = update.session_id.to_string();
        if parent == Some(id.as_str()) {
            return Err(SubagentError::Ownership);
        }
        if parent.is_some_and(|parent| !self.contains(parent)) {
            return Err(SubagentError::UnknownParent);
        }
        let child = self.0.entry(id.clone()).or_insert_with(|| Subagent {
            id,
            parent: parent.map(str::to_owned),
            title: None,
            description: None,
            state: None,
            capabilities: None,
        });
        if child.parent.as_deref() != parent {
            return Err(SubagentError::Ownership);
        }
        if !update.title.is_undefined() {
            child.title = update.title.clone().take();
        }
        if !update.description.is_undefined() {
            child.description = update.description.clone().take();
        }
        if !update.state.is_undefined() {
            child.state = update.state.clone().take();
        }
        if !update.capabilities.is_undefined() {
            child.capabilities = update.capabilities.clone().take();
        }
        Ok(())
    }

    /// Runtime termination does not imply that a remote child completed successfully.
    pub fn invalidate_activity(&mut self) {
        for child in self.0.values_mut() {
            child.state = None;
        }
    }
}

impl Subagent {
    pub fn is_active(&self) -> bool {
        matches!(
            self.state,
            Some(acp::StateUpdate::Running(_) | acp::StateUpdate::RequiresAction(_))
        )
    }
}

#[cfg(test)]
mod tests;
