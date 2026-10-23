use std::{fs, io::Write, path::PathBuf};

use futures::FutureExt;
use serde::{Deserialize, Serialize};

use crate::workspace::*;

#[cfg(test)]
mod tests;

const READ_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// A shared completion allows quit to await writes without tying their lifetime
/// to a window. Each successor waits for its predecessor, never on the UI thread.
pub(in crate::workspace) type PendingSave = futures::future::Shared<Task<()>>;

pub(in crate::workspace) fn enqueue(
    pending: &mut Option<PendingSave>,
    executor: &gpui_kit::BackgroundExecutor,
    write: impl FnOnce() + Send + 'static,
) {
    let previous = pending.take();
    let task = executor
        .spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            write();
        })
        .shared();
    // Closing or replacing the workspace must not cancel an accepted write.
    executor.spawn(task.clone()).detach();
    *pending = Some(task);
}

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    thread: Option<SessionId>,
    #[serde(default)]
    pinned: std::collections::BTreeSet<SessionId>,
    #[serde(default)]
    bookmarks: std::collections::BTreeSet<SessionId>,
    #[serde(default)]
    marker_bookmarks: std::collections::BTreeMap<SessionId, std::collections::BTreeSet<u64>>,
    #[serde(default)]
    seen_replies: std::collections::BTreeMap<SessionId, u64>,
    #[serde(default)]
    hidden_workspaces: std::collections::BTreeSet<Option<PathBuf>>,
}

impl Saved {
    fn merge_reads(&mut self, seen: &HashMap<SessionId, u64>) -> bool {
        let mut changed = false;
        for (&id, &cursor) in seen {
            if cursor > self.seen_replies.get(&id).copied().unwrap_or(0) {
                self.seen_replies.insert(id, cursor);
                changed = true;
            }
        }
        changed
    }

    fn forget_reads(&mut self, removed: &[SessionId]) -> bool {
        let mut changed = false;
        for id in removed {
            changed |= self.seen_replies.remove(id).is_some();
            changed |= self.marker_bookmarks.remove(id).is_some();
        }
        changed
    }

    fn set_marker_bookmarked(&mut self, session: SessionId, cursor: u64, bookmarked: bool) -> bool {
        if bookmarked {
            self.marker_bookmarks
                .entry(session)
                .or_default()
                .insert(cursor)
        } else if let Some(cursors) = self.marker_bookmarks.get_mut(&session) {
            let changed = cursors.remove(&cursor);
            if cursors.is_empty() {
                self.marker_bookmarks.remove(&session);
            }
            changed
        } else {
            false
        }
    }
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
    pub fn discover(server: &Target) -> Option<Self> {
        let root = bas::config::paths::state_root()?;
        Some(Self::open(
            root.join("basg")
                .join(format!("{}.json", server.state_key().ok()?)),
        ))
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

    pub fn bookmarks(&self) -> HashSet<SessionId> {
        self.saved.bookmarks.iter().copied().collect()
    }

    /// Stable original logical turn/page start cursors, not loaded item indexes.
    /// These are independent of the sidebar's thread-level bookmarks.
    pub fn marker_bookmarks(&self) -> HashMap<SessionId, HashSet<u64>> {
        self.saved
            .marker_bookmarks
            .iter()
            .map(|(&session, cursors)| (session, cursors.iter().copied().collect()))
            .collect()
    }

    /// Updates the local snapshot immediately and persists only this marker.
    /// Cursor zero is not a logical turn/page start and is ignored.
    pub fn queue_marker_bookmark(
        &mut self,
        pending: &mut Option<PendingSave>,
        executor: &gpui_kit::BackgroundExecutor,
        session: SessionId,
        cursor: u64,
        bookmarked: bool,
    ) {
        if cursor == 0 {
            return;
        }
        self.saved
            .set_marker_bookmarked(session, cursor, bookmarked);
        let path = self.path.clone();
        enqueue(pending, executor, move || {
            let mut store = Store {
                path,
                saved: Saved::default(),
            };
            if let Err(error) = store.save_marker_bookmark(session, cursor, bookmarked) {
                tracing::warn!(%error, "cannot save marker bookmark");
            }
        });
    }

    /// # Errors
    /// Returns lock, parse or write failures without replacing unrelated state.
    fn save_marker_bookmark(
        &mut self,
        session: SessionId,
        cursor: u64,
        bookmarked: bool,
    ) -> Result<(), Error> {
        if cursor == 0 {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        if saved.set_marker_bookmarked(session, cursor, bookmarked) {
            self.write(saved)
        } else {
            self.saved = saved;
            Ok(())
        }
    }

    /// # Errors
    /// Returns persistence failures without changing this or another window's saved state.
    pub fn set_bookmarked(&mut self, id: SessionId, bookmarked: bool) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        if bookmarked {
            saved.bookmarks.insert(id);
        } else {
            saved.bookmarks.remove(&id);
        }
        self.write(saved)
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
    #[cfg(test)]
    pub fn save_reads(&mut self, seen: &HashMap<SessionId, u64>) -> Result<(), Error> {
        if !seen
            .iter()
            .any(|(id, cursor)| *cursor > self.saved.seen_replies.get(id).copied().unwrap_or(0))
        {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        if saved.merge_reads(seen) {
            self.write(saved)
        } else {
            self.saved = saved;
            Ok(())
        }
    }

    /// Removes read cursors and marker bookmarks for deleted sessions.
    /// # Errors
    /// Returns persistence failures without altering unrelated saved state.
    pub fn forget_reads(&mut self, removed: &[SessionId]) -> Result<(), Error> {
        if removed.is_empty() {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        if saved.forget_reads(removed) {
            self.write(saved)
        } else {
            self.saved = saved;
            Ok(())
        }
    }

    fn queue_forget_reads(
        &mut self,
        pending: &mut Option<PendingSave>,
        executor: &gpui_kit::BackgroundExecutor,
        removed: &[SessionId],
    ) {
        if removed.is_empty() {
            return;
        }
        self.saved.forget_reads(removed);
        let path = self.path.clone();
        let removed = removed.to_vec();
        enqueue(pending, executor, move || {
            let mut store = Store {
                path,
                saved: Saved::default(),
            };
            if let Err(error) = store.forget_reads(&removed) {
                tracing::warn!(%error, "cannot remove saved read replies and marker bookmarks");
            }
        });
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
    #[cfg(test)]
    pub fn save(&mut self, thread: Option<SessionId>) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        saved.thread = thread;
        self.write(saved)
    }

    /// # Errors
    /// Returns lock, parse or write failures without replacing unrelated sections.
    fn save_navigation(
        &mut self,
        thread: Option<Option<SessionId>>,
        seen: &HashMap<SessionId, u64>,
    ) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut saved = self.latest()?;
        let mut changed = false;
        if let Some(thread) = thread {
            changed |= saved.thread != thread;
            saved.thread = thread;
        }
        changed |= saved.merge_reads(seen);
        if changed { self.write(saved) } else { Ok(()) }
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
                let _ = view.update(cx, |view, cx| view.persist_read_state(cx));
            }));
        }
    }

    pub(in crate::workspace) fn persist_read_state(&mut self, cx: &mut Context<Self>) {
        self.queue_navigation(None, cx);
    }

    pub(in crate::workspace) fn remember_thread(&mut self, cx: &mut Context<Self>) {
        self.queue_navigation(Some(self.selected.id), cx);
    }

    fn queue_navigation(&mut self, thread: Option<Option<SessionId>>, cx: &mut Context<Self>) {
        self.navigation.restore.read_save.take();
        let Some(store) = &mut self.navigation.restore.store else {
            return;
        };
        if let Some(thread) = thread {
            store.saved.thread = thread;
        }
        let path = store.path.clone();
        let seen = self.sessions.seen_replies.clone();
        enqueue(
            &mut self.navigation.restore.pending_save,
            cx.background_executor(),
            move || {
                let mut store = Store {
                    path,
                    saved: Saved::default(),
                };
                if let Err(error) = store.save_navigation(thread, &seen) {
                    tracing::warn!(%error, "cannot save navigation state");
                }
            },
        );
    }

    pub(in crate::workspace) fn forget_read_state(
        &mut self,
        removed: &[SessionId],
        cx: &mut Context<Self>,
    ) {
        let Some(store) = &mut self.navigation.restore.store else {
            return;
        };
        store.queue_forget_reads(
            &mut self.navigation.restore.pending_save,
            cx.background_executor(),
            removed,
        );
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
        // The catalog is newest first. Missing selections fall back to the most
        // recent live session before creating one.
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
