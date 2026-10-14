//! Connection-scoped user interactions with an attached frontend.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use bas::{acp, authentication::elicitation, model::SessionId};
use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};

use super::{Error, Host};

const MAX_PENDING: usize = 32;
const COMPLETION_HISTORY: usize = 64;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub pending: Vec<Pending>,
    pub completed_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub request_id: String,
    pub request: Value,
    pub answered: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ElicitationError {
    #[error("Elicitation request is no longer pending")]
    Expired,
    #[error(transparent)]
    Invalid(#[from] bas::authentication::Error),
}

#[derive(Default)]
pub struct Elicitations {
    state: Mutex<BTreeMap<SessionId, SessionInteractions>>,
}

struct SessionInteractions {
    viewers: usize,
    connection: Option<uuid::Uuid>,
    pending: BTreeMap<String, Interaction>,
    completed: VecDeque<String>,
    snapshots: watch::Sender<Snapshot>,
}

struct Interaction {
    pending: Pending,
    response: Option<oneshot::Sender<Value>>,
}

impl Default for SessionInteractions {
    fn default() -> Self {
        Self {
            viewers: 0,
            connection: None,
            pending: BTreeMap::new(),
            completed: VecDeque::new(),
            snapshots: watch::channel(Snapshot::default()).0,
        }
    }
}

impl SessionInteractions {
    fn publish(&self) {
        self.snapshots.send_replace(Snapshot {
            pending: self
                .pending
                .values()
                .map(|value| value.pending.clone())
                .collect(),
            completed_ids: self.completed.iter().cloned().collect(),
        });
    }

    fn cancel(&mut self) {
        for (_, interaction) in std::mem::take(&mut self.pending) {
            if let Some(response) = interaction.response {
                let _ = response.send(json!({"action":"cancel"}));
            }
        }
        self.completed.clear();
        self.publish();
    }
}

pub struct Subscription {
    elicitations: Arc<Elicitations>,
    session: SessionId,
    pub snapshots: watch::Receiver<Snapshot>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut state = self.elicitations.lock();
        if let Some(entry) = state.get_mut(&self.session)
            && self.snapshots.same_channel(&entry.snapshots.subscribe())
        {
            entry.viewers -= 1;
            if entry.viewers == 0 {
                entry.cancel();
                if entry.connection.is_none() {
                    state.remove(&self.session);
                }
            }
        }
    }
}

struct PendingGuard {
    elicitations: Arc<Elicitations>,
    session: SessionId,
    connection: uuid::Uuid,
    request_id: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let mut state = self.elicitations.lock();
        if let Some(entry) = state.get_mut(&self.session)
            && entry.connection == Some(self.connection)
            && entry
                .pending
                .get(&self.request_id)
                .is_some_and(|interaction| !interaction.pending.answered)
        {
            entry.pending.remove(&self.request_id);
            entry.publish();
        }
    }
}

impl Elicitations {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<SessionId, SessionInteractions>> {
        self.state
            .lock()
            .expect("elicitation state lock is not poisoned")
    }

    pub fn subscribe(self: &Arc<Self>, session: SessionId) -> Subscription {
        let mut state = self.lock();
        let entry = state.entry(session).or_default();
        entry.viewers += 1;
        Subscription {
            elicitations: Arc::clone(self),
            session,
            snapshots: entry.snapshots.subscribe(),
        }
    }

    pub fn open_connection(&self, session: SessionId, connection: uuid::Uuid) {
        let mut state = self.lock();
        let entry = state.entry(session).or_default();
        if entry.connection != Some(connection) {
            entry.connection = Some(connection);
            entry.cancel();
        }
    }

    /// # Errors
    /// Rejects expired requests and invalid answers without consuming the pending response.
    pub fn respond(
        &self,
        session: SessionId,
        request_id: &str,
        response: Value,
    ) -> Result<(), ElicitationError> {
        let mut state = self.lock();
        let entry = state.get_mut(&session).ok_or(ElicitationError::Expired)?;
        let interaction = entry
            .pending
            .get_mut(request_id)
            .ok_or(ElicitationError::Expired)?;
        if interaction.pending.answered {
            return Err(ElicitationError::Expired);
        }
        let request = elicitation::parse_request(&interaction.pending.request)?;
        elicitation::validate_response(&request, &response)?;
        let waiting_for_url = interaction
            .pending
            .request
            .get("mode")
            .and_then(Value::as_str)
            == Some("url")
            && response.get("action").and_then(Value::as_str) == Some("accept");
        let sender = interaction
            .response
            .take()
            .ok_or(ElicitationError::Expired)?;
        let sent = sender.send(response).is_ok();
        if waiting_for_url && sent {
            interaction.pending.answered = true;
        } else {
            entry.pending.remove(request_id);
        }
        entry.publish();
        sent.then_some(()).ok_or(ElicitationError::Expired)
    }

    pub fn complete(&self, session: SessionId, connection: uuid::Uuid, elicitation_id: &str) {
        let mut state = self.lock();
        let Some(entry) = state
            .get_mut(&session)
            .filter(|entry| entry.connection == Some(connection))
        else {
            return;
        };
        let request_id = entry.pending.iter().find_map(|(id, interaction)| {
            (interaction.pending.answered
                && interaction
                    .pending
                    .request
                    .get("elicitationId")
                    .and_then(Value::as_str)
                    == Some(elicitation_id))
            .then(|| id.clone())
        });
        if let Some(request_id) = request_id {
            entry.pending.remove(&request_id);
            entry.completed.push_back(request_id);
            while entry.completed.len() > COMPLETION_HISTORY {
                entry.completed.pop_front();
            }
            entry.publish();
        }
    }

    pub fn cancel(&self, session: SessionId) {
        if let Some(entry) = self.lock().get_mut(&session) {
            entry.cancel();
        }
    }

    pub fn cancel_connection(&self, session: SessionId, connection: uuid::Uuid) {
        let mut state = self.lock();
        if let Some(entry) = state.get_mut(&session)
            && entry.connection == Some(connection)
        {
            entry.connection = None;
            entry.cancel();
            if entry.viewers == 0 {
                state.remove(&session);
            }
        }
    }

    pub fn cancel_all(&self) {
        for entry in self.lock().values_mut() {
            entry.cancel();
        }
    }

    pub fn close(&self, session: SessionId) {
        if let Some(mut entry) = self.lock().remove(&session) {
            entry.cancel();
        }
    }

    /// # Errors
    /// Rejects unsupported requests and duplicate outstanding URL identifiers.
    pub async fn request(
        self: &Arc<Self>,
        session: SessionId,
        connection: uuid::Uuid,
        request: Value,
    ) -> Result<Value, ElicitationError> {
        let _ = elicitation::parse_request(&request)?;
        let (respond, response) = oneshot::channel();
        let request_id = uuid::Uuid::new_v4().to_string();
        {
            let mut state = self.lock();
            let Some(entry) = state
                .get_mut(&session)
                .filter(|entry| entry.viewers > 0 && entry.connection == Some(connection))
            else {
                return Ok(json!({"action":"cancel"}));
            };
            if entry.pending.len() >= MAX_PENDING {
                return Ok(json!({"action":"cancel"}));
            }
            if let Some(id) = request.get("elicitationId").and_then(Value::as_str)
                && entry.pending.values().any(|interaction| {
                    interaction
                        .pending
                        .request
                        .get("elicitationId")
                        .and_then(Value::as_str)
                        == Some(id)
                })
            {
                return Err(ElicitationError::Invalid(
                    bas::authentication::Error::Elicitation(
                        "duplicate outstanding URL identifier".into(),
                    ),
                ));
            }
            entry.pending.insert(
                request_id.clone(),
                Interaction {
                    pending: Pending {
                        request_id: request_id.clone(),
                        request,
                        answered: false,
                    },
                    response: Some(respond),
                },
            );
            entry.publish();
        }
        let _guard = PendingGuard {
            elicitations: Arc::clone(self),
            session,
            connection,
            request_id,
        };
        Ok(response
            .await
            .unwrap_or_else(|_| json!({"action":"cancel"})))
    }

    pub fn host(self: &Arc<Self>, fallback: Arc<dyn Host>) -> Arc<dyn Host> {
        Arc::new(InteractiveHost {
            elicitations: Arc::clone(self),
            fallback,
        })
    }
}

struct InteractiveHost {
    elicitations: Arc<Elicitations>,
    fallback: Arc<dyn Host>,
}

#[async_trait]
impl Host for InteractiveHost {
    fn open_elicitation_connection(&self, session: SessionId, connection: uuid::Uuid) {
        self.elicitations.open_connection(session, connection);
    }
    async fn elicit(
        &self,
        session: SessionId,
        connection: uuid::Uuid,
        request: Value,
    ) -> Result<Value, Error> {
        self.elicitations
            .request(session, connection, request)
            .await
            .map_err(|error| Error::with_source("host", "elicitation failed", error))
    }
    fn complete_elicitation(
        &self,
        session: SessionId,
        connection: uuid::Uuid,
        elicitation_id: &str,
    ) {
        self.elicitations
            .complete(session, connection, elicitation_id);
    }
    fn cancel_elicitations(&self, session: SessionId, connection: uuid::Uuid) {
        self.elicitations.cancel_connection(session, connection);
    }
    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> Result<acp::RequestPermissionOutcome, Error> {
        self.fallback.request_permission(request).await
    }
    async fn read_text_file(&self, request: acp::ReadTextFileRequest) -> Result<String, Error> {
        self.fallback.read_text_file(request).await
    }
    async fn write_text_file(&self, request: acp::WriteTextFileRequest) -> Result<(), Error> {
        self.fallback.write_text_file(request).await
    }
}

#[cfg(test)]
mod tests;
