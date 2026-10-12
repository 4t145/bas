use super::{SessionMeta, SessionStatus, TurnId};

#[cfg(test)]
mod tests;

/// A terminal failure of one runtime instance, not a persisted event or ACP result.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RuntimeFailure {
    #[error("session persistence failed: {message}")]
    Persistence { message: String },
}

/// Volatile state owned by one loaded session instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRuntime {
    /// Stable across snapshots; changes when the daemon loads a new instance.
    pub id: uuid::Uuid,
    pub failure: Option<RuntimeFailure>,
}

impl Default for SessionRuntime {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            failure: None,
        }
    }
}

/// Committed metadata and runtime health have independent lifetimes.
/// Only `meta` belongs in the session store.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionSnapshot {
    pub meta: SessionMeta,
    pub runtime: SessionRuntime,
}

impl From<SessionMeta> for SessionSnapshot {
    fn from(meta: SessionMeta) -> Self {
        Self {
            meta,
            runtime: SessionRuntime::default(),
        }
    }
}

impl SessionSnapshot {
    /// The observable status does not rewrite the last committed metadata.
    pub fn status(&self) -> SessionStatus {
        if self.runtime.failure.is_some() {
            SessionStatus::Failed
        } else {
            self.meta.status
        }
    }

    /// Runtime activity can end without changing the committed active turn.
    pub fn active_turn(&self) -> Option<TurnId> {
        if self.runtime.failure.is_some() {
            None
        } else {
            self.meta.active_turn
        }
    }
}
