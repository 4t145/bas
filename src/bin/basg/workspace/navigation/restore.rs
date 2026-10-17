use std::{fs, io::Write, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace::*;

#[cfg(test)]
mod tests;

const READ_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    thread: Option<SessionId>,
    #[serde(default)]
    pinned: std::collections::BTreeSet<SessionId>,
    #[serde(default)]
    seen_replies: std::collections::BTreeMap<SessionId, u64>,
    #[serde(default)]
    hidden_workspaces: std::collections::BTreeSet<Option<PathBuf>>,
}

pub(in crate::workspace) struct Store {
    path: PathBuf,
    saved: Saved,
}

struct WriteLock(fs::File);

impl Drop for WriteLock {
    fn drop(&mut self) {
        // A concurrently spawned process can retain a duplicate descriptor.
        // Finish the transaction without waiting for that process to close it.
        let _ = self.0.unlock();
    }
}

#[derive(Debug, thiserror::Error)]
pub(in crate::workspace) enum Error {
    #[error("cannot read or write window state: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid window state: {0}")]
    Json(#[from] serde_json::Error),
    #[error("window state is being updated by another process; please retry")]
    Busy,
}

impl Store {
    pub fn discover(server: SocketAddr) -> Option<Self> {
        let root = bas::config::paths::state_root()?;
        Some(Self::open(root.join("basg").join(format!(
            "{}.json",
            server.to_string().replace([':', '[', ']'], "_")
        ))))
    }

    pub fn open(path: PathBuf) -> Self {
        let saved = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, "cannot restore last Bas thread");
                Saved::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(error) => {
                tracing::warn!(%error, "cannot read BasG window state");
                Saved::default()
            }
        };
        Self { path, saved }
    }

    pub fn thread(&self) -> Option<SessionId> {
        self.saved.thread
    }

    pub fn hidden_workspaces(&self) -> HashSet<Option<PathBuf>> {
        self.saved.hidden_workspaces.iter().cloned().collect()
    }

    /// # Errors
    /// Returns lock, read or write failures without altering unrelated window state.
    pub fn set_workspace_hidden(
        &mut self,
        directory: Option<PathBuf>,
        hidden: bool,
    ) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        if hidden {
            saved.hidden_workspaces.insert(directory);
        } else {
            saved.hidden_workspaces.remove(&directory);
        }
        self.write(saved)
    }

    pub fn pinned(&self) -> HashSet<SessionId> {
        self.saved.pinned.iter().copied().collect()
    }

    pub fn seen_replies(&self) -> HashMap<SessionId, u64> {
        self.saved
            .seen_replies
            .iter()
            .map(|(&id, &cursor)| (id, cursor))
            .collect()
    }

    /// # Errors
    /// Returns lock, read or write failures without replacing corrupt state.
    pub fn save_reads(&mut self, seen: &HashMap<SessionId, u64>) -> Result<(), Error> {
        if !seen
            .iter()
            .any(|(id, cursor)| *cursor > self.saved.seen_replies.get(id).copied().unwrap_or(0))
        {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        let mut changed = false;
        for (&id, &cursor) in seen {
            if cursor > saved.seen_replies.get(&id).copied().unwrap_or(0) {
                saved.seen_replies.insert(id, cursor);
                changed = true;
            }
        }
        if changed {
            self.write(saved)
        } else {
            self.saved = saved;
            Ok(())
        }
    }

    /// # Errors
    /// Returns persistence failures without altering unrelated saved state.
    pub fn forget_reads(&mut self, removed: &[SessionId]) -> Result<(), Error> {
        if removed.is_empty() {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        let mut changed = false;
        for id in removed {
            changed |= saved.seen_replies.remove(id).is_some();
        }
        if changed {
            self.write(saved)
        } else {
            self.saved = saved;
            Ok(())
        }
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
    fn lock(&self) -> Result<WriteLock, Error> {
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
            Ok(()) => Ok(WriteLock(lock)),
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
    pub(in crate::workspace) fn attach_restore(&mut self, store: Option<Store>) {
        if let Some(store) = &store {
            for (id, cursor) in store.seen_replies() {
                let seen = self.sessions.seen_replies.entry(id).or_default();
                *seen = (*seen).max(cursor);
            }
        }
        self.navigation.attach_restore(store);
    }

    pub(in crate::workspace) fn see_reply(
        &mut self,
        session: SessionId,
        cursor: u64,
        cx: &mut Context<Self>,
    ) {
        let seen = self.sessions.seen_replies.entry(session).or_default();
        if cursor <= *seen {
            return;
        }
        *seen = cursor;
        if self.navigation.restore.store.is_some() {
            self.navigation.restore.read_save = Some(cx.spawn(async move |view, cx| {
                cx.background_executor().timer(READ_SAVE_DELAY).await;
                let _ = view.update(cx, |view, _| view.persist_read_state());
            }));
        }
    }

    pub(in crate::workspace) fn persist_read_state(&mut self) {
        self.navigation.restore.read_save.take();
        if let Some(store) = &mut self.navigation.restore.store
            && let Err(error) = store.save_reads(&self.sessions.seen_replies)
        {
            tracing::warn!(%error, "cannot save read replies");
        }
    }

    pub(in crate::workspace) fn remember_thread(&mut self) {
        self.persist_read_state();
        if let Some(store) = &mut self.navigation.restore.store
            && let Err(error) = store.save(self.selected.id)
        {
            tracing::warn!(%error, "cannot save last Bas thread");
        }
    }

    pub(in crate::workspace) fn restore_thread(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !std::mem::take(&mut self.navigation.restore.on_connect)
            || self.selected.id.is_some()
            || self.selected.creating
        {
            return;
        }
        let saved = self
            .navigation
            .restore
            .store
            .as_ref()
            .and_then(Store::thread);
        // The catalog is newest first. A removed thread (or a first launch after
        // upgrading) falls back to the most recent live session before creating one.
        let thread = saved
            .filter(|id| {
                self.sessions
                    .entries
                    .iter()
                    .any(|session| session.id == *id && !session.archived)
            })
            .or_else(|| {
                self.sessions
                    .entries
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
        } else if self
            .navigation
            .hidden
            .contains(&self.navigation.selected_directory)
        {
            // Do not recreate a removed workspace merely because it is the launch cwd.
            self.detach_thread(None, window, cx);
        }
    }
}
