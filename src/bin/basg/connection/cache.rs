//! Disposable, connection-scoped history. Only safe server windows establish
//! page boundaries; context never fills holes in replayable history.

use std::path::{Path, PathBuf};

use bas::{
    model::{Event, SessionId},
    service::{HistoryWindow, tls::ConnectionOptions},
};
use tokio::sync::{mpsc, oneshot};

mod store;
#[cfg(test)]
mod tests;

const DEFAULT_BYTE_LIMIT: u64 = 512 * 1024 * 1024;
const QUEUE_CAPACITY: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] toasty::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid history cache: {0}")]
    Invalid(&'static str),
    #[error("history cache worker stopped")]
    WorkerClosed,
}

type Reply<T> = oneshot::Sender<Result<T, Error>>;

enum Command {
    Latest(SessionId, Reply<Option<HistoryWindow>>),
    Before(SessionId, u64, Reply<Option<HistoryWindow>>),
    Store(Box<HistoryWindow>, Reply<()>),
    Append(SessionId, Vec<Event>, Reply<()>),
    Remove(SessionId, Reply<()>),
}

impl Command {
    async fn run(self, store: &store::Store) {
        match self {
            Self::Latest(id, reply) => {
                let _ = reply.send(store.load(id, None).await);
            }
            Self::Before(id, before, reply) => {
                let _ = reply.send(store.load(id, Some(before)).await);
            }
            Self::Store(window, reply) => {
                let _ = reply.send(store.store(&window).await);
            }
            Self::Append(id, events, reply) => {
                let _ = reply.send(store.append(id, &events).await);
            }
            Self::Remove(id, reply) => {
                let _ = reply.send(store.remove(id).await);
            }
        }
    }
}

/// Bounded request queue; no histories are retained by this handle.
#[derive(Clone)]
pub struct Cache {
    requests: mpsc::Sender<Command>,
}

impl Cache {
    /// Opens the private GUI cache, never the daemon database.
    ///
    /// # Errors
    /// Returns path, schema, database, or worker startup failures.
    pub async fn open(options: &ConnectionOptions) -> Result<Self, Error> {
        let root = dirs::cache_dir().ok_or(Error::Invalid("no cache directory"))?;
        Self::open_at(
            root.join("basg/history-v1/history.sqlite"),
            options,
            DEFAULT_BYTE_LIMIT,
        )
        .await
    }

    /// The limit bounds serialized content plus row overhead across all namespaces.
    /// SQLite has additional fixed schema/index overhead, and reclaims freed pages.
    ///
    /// # Errors
    /// Rejects symlinks, nonprivate paths, incompatible schemas and database errors.
    pub async fn open_at(
        path: impl AsRef<Path>,
        options: &ConnectionOptions,
        byte_limit: u64,
    ) -> Result<Self, Error> {
        let path: PathBuf = path.as_ref().to_owned();
        let options = options.clone();
        let (requests, mut incoming) = mpsc::channel::<Command>(QUEUE_CAPACITY);
        let (started, ready) = oneshot::channel();
        std::thread::Builder::new()
            .name("basg-history-cache".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = started.send(Err(Error::Io(error)));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let store = match store::Store::open(&path, &options, byte_limit).await {
                        Ok(store) => store,
                        Err(error) => {
                            let _ = started.send(Err(error));
                            return;
                        }
                    };
                    if started.send(Ok(())).is_err() {
                        return;
                    }
                    while let Some(command) = incoming.recv().await {
                        command.run(&store).await;
                    }
                });
            })?;
        ready.await.map_err(|_| Error::WorkerClosed)??;
        Ok(Self { requests })
    }

    async fn request<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, Error> {
        let (reply, response) = oneshot::channel();
        self.requests
            .send(command(reply))
            .await
            .map_err(|_| Error::WorkerClosed)?;
        response.await.map_err(|_| Error::WorkerClosed)?
    }

    /// Loads one safe newest window and its contiguous live extension.
    /// Its `cursor` is the retained replay frontier, which can trail metadata
    /// observed during an older-page fetch. Resume/catch up from `cursor`, not
    /// `session.meta.revision`.
    /// # Errors
    /// Returns storage or corrupt-cache errors.
    pub async fn load_latest(&self, session: SessionId) -> Result<Option<HistoryWindow>, Error> {
        self.request(|reply| Command::Latest(session, reply)).await
    }

    /// Loads a saved page or covering-page prefix ending at `before - 1`.
    /// # Errors
    /// Returns storage or corrupt-cache errors.
    pub async fn load_before(
        &self,
        session: SessionId,
        before: u64,
    ) -> Result<Option<HistoryWindow>, Error> {
        self.request(|reply| Command::Before(session, before, reply))
            .await
    }

    /// Atomically retains a safe window, without moving the newest frontier back.
    /// Oversized sessions are discarded rather than interrupting the live flow.
    /// # Errors
    /// Returns invalid-window, storage or corrupt-cache errors.
    pub async fn store_window(&self, window: &HistoryWindow) -> Result<(), Error> {
        self.request(|reply| Command::Store(Box::new(window.clone()), reply))
            .await
    }

    /// Ignores transient events and duplicates; stops at the first replay gap.
    /// # Errors
    /// Returns mismatched-session, storage or corrupt-cache errors.
    pub async fn append(&self, session: SessionId, events: &[Event]) -> Result<(), Error> {
        self.request(|reply| Command::Append(session, events.to_vec(), reply))
            .await
    }

    /// Removes every retained page and event for this connection's session.
    /// # Errors
    /// Returns database errors.
    pub async fn remove(&self, session: SessionId) -> Result<(), Error> {
        self.request(|reply| Command::Remove(session, reply)).await
    }
}
