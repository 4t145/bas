//! Keep Toasty's SQLite connection tasks off the daemon's Tokio workers.
//!
//! Toasty's SQLite driver uses synchronous SQLite calls within its connection
//! task. A dedicated current-thread runtime confines lock waits to this OS
//! thread while callers await replies without blocking the daemon runtime.

use std::path::Path;

use bas::model::{AgentId, Cursor, Event, SessionId, SessionMeta, TurnId};
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};

use super::{Error, Page, PromptAcceptance, ResumeData, SqliteStore, StoredSession};
use crate::storage::Storage;
use crate::storage::Window;

type Reply<T> = oneshot::Sender<Result<T, Error>>;

enum Command {
    LastUsedAgent(Reply<Option<AgentId>>),
    RecordCreatedSession(SessionId, Reply<()>),
    Create(SessionMeta, Option<ResumeData>, Reply<()>),
    Commit(SessionMeta, Vec<Event>, Option<PromptAcceptance>, Reply<()>),
    Acceptance(SessionId, TurnId, Reply<Option<PromptAcceptance>>),
    SaveResume(SessionId, Option<ResumeData>, Reply<()>),
    Load(SessionId, Reply<Option<StoredSession>>),
    List(Reply<Vec<SessionMeta>>),
    Page(SessionId, Option<Cursor>, usize, Reply<Page>),
    Window(SessionId, Option<Cursor>, usize, Reply<Window>),
    Tail(SessionId, usize, Reply<Vec<Event>>),
    Delete(SessionId, Reply<bool>),
    #[cfg(test)]
    TestSql(TestSql, Reply<()>),
}

impl Command {
    async fn run(self, store: &SqliteStore) {
        match self {
            Self::RecordCreatedSession(id, reply) => {
                let _ = reply.send(store.record_created_session(id).await);
            }
            Self::LastUsedAgent(reply) => {
                let _ = reply.send(store.last_used_agent().await);
            }
            Self::Create(meta, resume, reply) => {
                let _ = reply.send(store.create(meta, resume).await);
            }
            Self::Commit(meta, events, acceptance, reply) => {
                let _ = reply.send(store.commit(meta, &events, acceptance).await);
            }
            Self::Acceptance(session, turn, reply) => {
                let _ = reply.send(store.acceptance(session, turn).await);
            }
            Self::SaveResume(id, resume, reply) => {
                let _ = reply.send(store.save_resume(id, resume).await);
            }
            Self::Load(id, reply) => {
                let _ = reply.send(store.load(id).await);
            }
            Self::List(reply) => {
                let _ = reply.send(store.list().await);
            }
            Self::Page(id, after, limit, reply) => {
                let _ = reply.send(store.page(id, after, limit).await);
            }
            Self::Window(id, before, limit, reply) => {
                let _ = reply.send(store.window(id, before, limit).await);
            }
            Self::Tail(id, count, reply) => {
                let _ = reply.send(store.tail(id, count).await);
            }
            Self::Delete(id, reply) => {
                let _ = reply.send(store.delete(id).await);
            }
            #[cfg(test)]
            Self::TestSql(sql, reply) => {
                let _ = reply.send(sql.run(store).await);
            }
        }
    }
}

/// A handle that communicates with one long-lived, dedicated SQLite runtime.
///
/// Dropping the last handle closes the bounded channel and lets the runtime
/// finish queued operations, close the database and exit its thread.
pub struct LocalDb {
    requests: mpsc::Sender<Command>,
}

impl LocalDb {
    /// Starts an isolated SQLite runtime and waits for schema migrations.
    ///
    /// # Errors
    /// Fails when opening the database, migrating its schema or starting the
    /// dedicated OS thread fails.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let (requests, mut incoming) = mpsc::channel::<Command>(64);
        let (started, ready) = oneshot::channel();
        std::thread::Builder::new()
            .name("bas-sqlite".into())
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
                    let store = match SqliteStore::open(path).await {
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
        let (reply, result) = oneshot::channel();
        self.requests
            .send(command(reply))
            .await
            .map_err(|_| Error::WorkerClosed)?;
        result.await.map_err(|_| Error::WorkerClosed)?
    }

    #[cfg(test)]
    pub(in crate::storage) async fn test_sql(&self, command: TestSql) -> Result<(), Error> {
        self.request(move |reply| Command::TestSql(command, reply))
            .await
    }
}

impl Storage for LocalDb {
    fn record_created_session(&self, id: SessionId) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(self.request(move |reply| Command::RecordCreatedSession(id, reply)))
    }

    fn last_used_agent(&self) -> BoxFuture<'_, Result<Option<AgentId>, Error>> {
        Box::pin(self.request(Command::LastUsedAgent))
    }

    fn create(
        &self,
        meta: SessionMeta,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(self.request(move |reply| Command::Create(meta, resume, reply)))
    }

    fn commit<'a>(
        &'a self,
        meta: SessionMeta,
        events: &'a [Event],
        acceptance: Option<PromptAcceptance>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        let events = events.to_vec();
        Box::pin(self.request(move |reply| Command::Commit(meta, events, acceptance, reply)))
    }

    fn acceptance(
        &self,
        session: SessionId,
        turn: TurnId,
    ) -> BoxFuture<'_, Result<Option<PromptAcceptance>, Error>> {
        Box::pin(self.request(move |reply| Command::Acceptance(session, turn, reply)))
    }

    fn save_resume(
        &self,
        id: SessionId,
        resume: Option<ResumeData>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(self.request(move |reply| Command::SaveResume(id, resume, reply)))
    }

    fn load(&self, id: SessionId) -> BoxFuture<'_, Result<Option<StoredSession>, Error>> {
        Box::pin(self.request(move |reply| Command::Load(id, reply)))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<SessionMeta>, Error>> {
        Box::pin(self.request(Command::List))
    }

    fn page(
        &self,
        id: SessionId,
        after: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Page, Error>> {
        Box::pin(self.request(move |reply| Command::Page(id, after, limit, reply)))
    }

    fn window(
        &self,
        id: SessionId,
        before: Option<Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Window, Error>> {
        Box::pin(self.request(move |reply| Command::Window(id, before, limit, reply)))
    }

    fn tail(&self, id: SessionId, count: usize) -> BoxFuture<'_, Result<Vec<Event>, Error>> {
        Box::pin(self.request(move |reply| Command::Tail(id, count, reply)))
    }

    fn delete(&self, id: SessionId) -> BoxFuture<'_, Result<bool, Error>> {
        Box::pin(self.request(move |reply| Command::Delete(id, reply)))
    }

    fn is_persistent(&self) -> bool {
        true
    }
}

#[cfg(test)]
pub(in crate::storage) enum TestSql {
    RejectThirdEvent,
    RejectSelection,
    ClearHistoryIndex,
    CorruptFirstPayload,
}

#[cfg(test)]
impl TestSql {
    async fn run(self, store: &SqliteStore) -> Result<(), Error> {
        use toasty::sql;

        let mut db = store.db.clone();
        match self {
            Self::RejectSelection => {
                sql::statement("CREATE TRIGGER fail_selection BEFORE INSERT ON agent_selection BEGIN SELECT RAISE(ABORT, 'failure'); END")
                    .exec(&mut db)
                    .await?;
            }
            Self::ClearHistoryIndex => {
                for table in [
                    "history_index",
                    "history_dependencies",
                    "history_context_keys",
                    "history_turns",
                ] {
                    sql::statement(format!("DELETE FROM {table}"))
                        .exec(&mut db)
                        .await?;
                }
            }
            Self::CorruptFirstPayload => {
                sql::statement("UPDATE events SET payload = X'00' WHERE cursor = 1")
                    .exec(&mut db)
                    .await?;
            }
            Self::RejectThirdEvent => {
                sql::statement("CREATE TRIGGER fail_third BEFORE INSERT ON events WHEN NEW.cursor = 3 BEGIN SELECT RAISE(ABORT, 'failure'); END")
                    .exec(&mut db)
                    .await?;
            }
        }
        Ok(())
    }
}
