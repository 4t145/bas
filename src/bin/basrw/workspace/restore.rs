use std::{fs, io::Write, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::*;

#[cfg(test)]
mod tests;

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    thread: Option<SessionId>,
    #[serde(default)]
    pinned: std::collections::BTreeSet<SessionId>,
}

pub(super) struct Store {
    path: PathBuf,
    saved: Saved,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("cannot read or write window state: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid window state: {0}")]
    Json(#[from] serde_json::Error),
    #[error("window state is being updated by another process; please retry")]
    Busy,
}

impl Store {
    pub fn discover(server: SocketAddr) -> Option<Self> {
        let root = basr::config::paths::state_root()?;
        Some(Self::open(root.join("basrw").join(format!(
            "{}.json",
            server.to_string().replace([':', '[', ']'], "_")
        ))))
    }

    pub fn open(path: PathBuf) -> Self {
        let saved = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, "cannot restore last BASR thread");
                Saved::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(error) => {
                tracing::warn!(%error, "cannot read BASR window state");
                Saved::default()
            }
        };
        Self { path, saved }
    }

    pub fn thread(&self) -> Option<SessionId> {
        self.saved.thread
    }

    pub fn pinned(&self) -> HashSet<SessionId> {
        self.saved.pinned.iter().copied().collect()
    }

    /// # Errors
    /// Returns read/parse failures rather than overwriting corrupt saved state.
    fn latest(&self) -> Result<Saved, Error> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// # Errors
    /// Returns persistence failures without changing the in-memory pin set.
    pub fn set_pinned(&mut self, id: SessionId, pinned: bool) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        if pinned {
            saved.pinned.insert(id);
        } else {
            saved.pinned.remove(&id);
        }
        self.write(saved)
    }

    /// # Errors
    /// Returns filesystem or serialization failures; the previous state remains intact.
    pub fn save(&mut self, thread: Option<SessionId>) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        saved.thread = thread;
        self.write(saved)
    }

    /// # Errors
    /// Returns IO failures or Busy without blocking the UI on another writer.
    fn lock(&self) -> Result<fs::File, Error> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.with_extension("lock"))?;
        match lock.try_lock() {
            Ok(()) => Ok(lock),
            Err(fs::TryLockError::WouldBlock) => Err(Error::Busy),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// # Errors
    /// Returns filesystem or serialization failures; the previous state remains intact.
    fn write(&mut self, saved: Saved) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&saved)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<(), Error> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result?;
        self.saved = saved;
        Ok(())
    }
}

impl Workspace {
    pub(super) fn remember_thread(&mut self) {
        if let Some(store) = &mut self.restore_store
            && let Err(error) = store.save(self.active)
        {
            tracing::warn!(%error, "cannot save last BASR thread");
        }
    }

    pub(super) fn restore_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.restore_on_connect) || self.active.is_some() || self.creating {
            return;
        }
        let saved = self.restore_store.as_ref().and_then(Store::thread);
        // The catalog is newest first. A removed thread (or a first launch after
        // upgrading) falls back to the most recent live session before creating one.
        let thread = saved
            .filter(|id| {
                self.sessions
                    .iter()
                    .any(|session| session.id == *id && !session.archived)
            })
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|session| {
                        !session.archived
                            && matches!(
                                session.status,
                                SessionStatus::Idle | SessionStatus::Working
                            )
                    })
                    .map(|session| session.id)
            });
        if let Some(thread) = thread {
            self.activate(thread, true, window, cx);
        }
    }
}
