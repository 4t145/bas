use std::{fs, io::Write, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::*;

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    thread: Option<SessionId>,
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
}

impl Store {
    pub fn discover(server: SocketAddr) -> Option<Self> {
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })?;
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

    /// # Errors
    /// Returns filesystem or serialization failures; the previous state remains intact.
    pub fn save(&mut self, thread: Option<SessionId>) -> Result<(), Error> {
        let saved = Saved { thread };
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
            .filter(|id| self.sessions.iter().any(|session| session.id == *id))
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|session| {
                        matches!(session.status, SessionStatus::Idle | SessionStatus::Working)
                    })
                    .map(|session| session.id)
            });
        if let Some(thread) = thread {
            self.activate(thread, true, window, cx);
        }
    }
}
