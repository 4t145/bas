use std::collections::HashMap;

use bas::model::{RuntimeFailure, SessionId, SessionRuntime, SessionSnapshot};

/// Catalog is serial; other RPC responses may belong to an older loaded instance.
#[derive(Default)]
pub(super) struct Runtimes(HashMap<SessionId, SessionRuntime>);

impl Runtimes {
    pub fn observe(&mut self, snapshot: &SessionSnapshot, catalog: bool) {
        let incoming = &snapshot.runtime;
        let current = self
            .0
            .entry(snapshot.meta.id)
            .or_insert_with(|| incoming.clone());
        if current.id != incoming.id {
            if catalog {
                *current = incoming.clone();
            }
        } else if current.failure.is_none() {
            current.failure = incoming.failure.clone();
        }
    }

    pub fn failure(&self, session: SessionId) -> Option<&RuntimeFailure> {
        self.0
            .get(&session)
            .and_then(|runtime| runtime.failure.as_ref())
    }

    pub fn remove(&mut self, session: SessionId) {
        self.0.remove(&session);
    }
}

#[cfg(test)]
mod tests;
