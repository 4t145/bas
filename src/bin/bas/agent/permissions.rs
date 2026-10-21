//! Routes live ACP approvals to connected clients without changing headless policy.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use bas::{
    acp,
    model::{PendingPermission, SessionId},
};
use tokio::sync::{oneshot, watch};

use super::{Error, Host};

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("permission request is no longer pending")]
    Expired,
    #[error("permission option was not offered by the agent")]
    InvalidOption,
}

#[derive(Default)]
pub struct Permissions {
    state: Mutex<BTreeMap<SessionId, SessionPermissions>>,
}

struct SessionPermissions {
    viewers: usize,
    pending: BTreeMap<String, Pending>,
    snapshots: watch::Sender<Vec<PendingPermission>>,
}

struct Pending {
    permission: PendingPermission,
    respond: oneshot::Sender<acp::RequestPermissionOutcome>,
}

impl Default for SessionPermissions {
    fn default() -> Self {
        Self {
            viewers: 0,
            pending: BTreeMap::new(),
            snapshots: watch::channel(Vec::new()).0,
        }
    }
}

impl SessionPermissions {
    fn publish(&self) {
        self.snapshots.send_replace(
            self.pending
                .values()
                .map(|pending| pending.permission.clone())
                .collect(),
        );
    }

    fn cancel(&mut self) {
        for (_, pending) in std::mem::take(&mut self.pending) {
            let _ = pending
                .respond
                .send(acp::RequestPermissionOutcome::Cancelled);
        }
        self.publish();
    }
}

/// Keeps interactive approval active only while its subscription is alive.
pub struct PermissionSubscription {
    permissions: Arc<Permissions>,
    session: SessionId,
    pub snapshots: watch::Receiver<Vec<PendingPermission>>,
}

impl Drop for PermissionSubscription {
    fn drop(&mut self) {
        let mut state = self.permissions.lock();
        if let Some(session) = state.get_mut(&self.session)
            && self.snapshots.same_channel(&session.snapshots.subscribe())
        {
            session.viewers -= 1;
            if session.viewers == 0 {
                session.cancel();
            }
        }
    }
}

/// Removes a pending request even if its backend callback is aborted.
struct PendingGuard {
    permissions: Arc<Permissions>,
    session: SessionId,
    request_id: String,
}

struct PendingResponse {
    _guard: PendingGuard,
    response: oneshot::Receiver<acp::RequestPermissionOutcome>,
}

enum PermissionRoute {
    Headless,
    Detached,
    Pending(PendingResponse),
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let mut state = self.permissions.lock();
        if let Some(session) = state.get_mut(&self.session)
            && session.pending.remove(&self.request_id).is_some()
        {
            session.publish();
        }
    }
}

impl Permissions {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<SessionId, SessionPermissions>> {
        self.state
            .lock()
            .expect("permission state lock is not poisoned")
    }

    pub fn subscribe(self: &Arc<Self>, session: SessionId) -> PermissionSubscription {
        let mut state = self.lock();
        let entry = state.entry(session).or_default();
        entry.viewers += 1;
        PermissionSubscription {
            permissions: Arc::clone(self),
            session,
            snapshots: entry.snapshots.subscribe(),
        }
    }

    /// # Errors
    /// Rejects unknown requests and options without consuming a valid pending request.
    pub fn respond(
        &self,
        session: SessionId,
        request_id: &str,
        outcome: acp::RequestPermissionOutcome,
    ) -> Result<(), PermissionError> {
        let mut state = self.lock();
        let entry = state.get_mut(&session).ok_or(PermissionError::Expired)?;
        let pending = entry
            .pending
            .get(request_id)
            .ok_or(PermissionError::Expired)?;
        if let acp::RequestPermissionOutcome::Selected(selected) = &outcome
            && !pending
                .permission
                .request
                .options
                .iter()
                .any(|option| option.option_id == selected.option_id)
        {
            return Err(PermissionError::InvalidOption);
        }
        let pending = entry
            .pending
            .remove(request_id)
            .expect("validated pending request");
        let sent = pending.respond.send(outcome).is_ok();
        entry.publish();
        sent.then_some(()).ok_or(PermissionError::Expired)
    }

    pub fn cancel(&self, session: SessionId) {
        if let Some(entry) = self.lock().get_mut(&session) {
            entry.cancel();
        }
    }

    pub fn has_pending(&self, session: SessionId) -> bool {
        self.lock()
            .get(&session)
            .is_some_and(|entry| !entry.pending.is_empty())
    }

    pub fn cancel_all(&self) {
        for entry in self.lock().values_mut() {
            entry.cancel();
        }
    }

    /// Closes viewers and cancels requests when a session is removed.
    pub fn close(&self, session: SessionId) {
        if let Some(mut entry) = self.lock().remove(&session) {
            entry.cancel();
        }
    }

    pub fn host(self: &Arc<Self>, fallback: Arc<dyn Host>) -> Arc<dyn Host> {
        Arc::new(InteractiveHost {
            permissions: Arc::clone(self),
            fallback,
        })
    }

    fn enqueue(
        self: &Arc<Self>,
        session: SessionId,
        request: acp::RequestPermissionRequest,
    ) -> PermissionRoute {
        let mut state = self.lock();
        let Some(entry) = state.get_mut(&session) else {
            return PermissionRoute::Headless;
        };
        if entry.viewers == 0 {
            return PermissionRoute::Detached;
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        let (respond, response) = oneshot::channel();
        let permission = PendingPermission {
            request_id: request_id.clone(),
            request,
        };
        entry.pending.insert(
            request_id.clone(),
            Pending {
                permission,
                respond,
            },
        );
        entry.publish();
        PermissionRoute::Pending(PendingResponse {
            _guard: PendingGuard {
                permissions: Arc::clone(self),
                session,
                request_id,
            },
            response,
        })
    }
}

struct InteractiveHost {
    permissions: Arc<Permissions>,
    fallback: Arc<dyn Host>,
}

#[async_trait]
impl Host for InteractiveHost {
    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> Result<acp::RequestPermissionOutcome, Error> {
        let session: SessionId = request
            .session_id
            .to_string()
            .parse()
            .map_err(|error| Error::with_source("host", "invalid permission session", error))?;
        match self.permissions.enqueue(session, request.clone()) {
            PermissionRoute::Pending(pending) => Ok(pending
                .response
                .await
                .unwrap_or(acp::RequestPermissionOutcome::Cancelled)),
            PermissionRoute::Detached => Ok(acp::RequestPermissionOutcome::Cancelled),
            PermissionRoute::Headless => self.fallback.request_permission(request).await,
        }
    }

    async fn read_text_file(&self, request: acp::ReadTextFileRequest) -> Result<String, Error> {
        self.fallback.read_text_file(request).await
    }

    async fn write_text_file(&self, request: acp::WriteTextFileRequest) -> Result<(), Error> {
        self.fallback.write_text_file(request).await
    }
}
