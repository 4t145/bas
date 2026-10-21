use std::path::Path;

use bas::{
    model::{Event, SessionId, SessionSnapshot},
    service::{HistoryWindow, tls::ConnectionOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toasty::{Transaction, sql, stmt::Value};

use super::Error;

mod paths;

// This application id distinguishes the disposable GUI cache from daemon data.
const APPLICATION_ID: i64 = 0x42474348;
const SCHEMA_VERSION: i64 = 1;
const ROW_OVERHEAD: i64 = 256;

pub(super) struct Store {
    db: toasty::Db,
    namespace: String,
    limit: i64,
}

#[derive(Serialize, Deserialize)]
struct Boundary {
    start: u64,
    before: Option<u64>,
    context: Vec<Event>,
}

struct SavedSession {
    snapshot: SessionSnapshot,
    frontier: u64,
}

impl Store {
    pub(super) async fn open(
        path: &Path,
        options: &ConnectionOptions,
        limit: u64,
    ) -> Result<Self, Error> {
        paths::prepare(path)?;
        let namespace = namespace(options)?;
        let mut db = toasty::Db::builder()
            .max_pool_size(1)
            .build(toasty_driver_sqlite::Sqlite::open(path))
            .await?;
        sql::query("PRAGMA busy_timeout = 5000")
            .exec(&mut db)
            .await?;
        let app = scalar(sql::query("PRAGMA application_id").exec(&mut db).await?)?;
        let version = scalar(sql::query("PRAGMA user_version").exec(&mut db).await?)?;
        if app == 0 && version == 0 {
            let tables = sql::query("SELECT name FROM sqlite_master WHERE type = 'table'")
                .exec(&mut db)
                .await?;
            if !tables.is_empty() {
                return Err(Error::Invalid("not an empty cache database"));
            }
            sql::statement("PRAGMA auto_vacuum = FULL")
                .exec(&mut db)
                .await?;
            let mut tx = db.transaction().await?;
            for statement in [
                "CREATE TABLE IF NOT EXISTS cache_sessions (namespace TEXT NOT NULL, session TEXT NOT NULL, snapshot BLOB NOT NULL, frontier INTEGER NOT NULL, touched INTEGER NOT NULL, bytes INTEGER NOT NULL, PRIMARY KEY(namespace, session))",
                "CREATE TABLE IF NOT EXISTS cache_pages (namespace TEXT NOT NULL, session TEXT NOT NULL, upper INTEGER NOT NULL, boundary BLOB NOT NULL, PRIMARY KEY(namespace, session, upper))",
                "CREATE TABLE IF NOT EXISTS cache_events (namespace TEXT NOT NULL, session TEXT NOT NULL, cursor INTEGER NOT NULL, payload BLOB NOT NULL, PRIMARY KEY(namespace, session, cursor))",
                "CREATE INDEX IF NOT EXISTS cache_lru ON cache_sessions(touched)",
            ] {
                sql::statement(statement).exec(&mut tx).await?;
            }
            sql::statement(format!("PRAGMA application_id = {APPLICATION_ID}"))
                .exec(&mut tx)
                .await?;
            sql::statement(format!("PRAGMA user_version = {SCHEMA_VERSION}"))
                .exec(&mut tx)
                .await?;
            tx.commit().await?;
        } else if app != APPLICATION_ID || version != SCHEMA_VERSION {
            return Err(Error::Invalid("incompatible cache schema"));
        }
        let store = Self {
            db,
            namespace,
            limit: i64::try_from(limit).unwrap_or(i64::MAX),
        };
        let mut db = store.db.clone();
        let mut tx = transaction(&mut db).await?;
        store.evict(&mut tx, None).await?;
        tx.commit().await?;
        Ok(store)
    }

    pub(super) async fn load(
        &self,
        id: SessionId,
        before: Option<u64>,
    ) -> Result<Option<HistoryWindow>, Error> {
        if before == Some(0) {
            return Ok(None);
        }
        let mut db = self.db.clone();
        let mut tx = transaction(&mut db).await?;
        let Some(SavedSession {
            snapshot: session,
            frontier,
        }) = self.session(&mut tx, id).await?
        else {
            return Ok(None);
        };
        let requested = before.map(|before| before - 1);
        let rows = match requested {
            // Resync can move the newest page's start inside an older saved
            // page. Reuse its prefix, retaining that page's safe start/context.
            Some(upper) => sql::query("SELECT upper, boundary FROM cache_pages WHERE namespace = ?1 AND session = ?2 AND upper >= ?3 AND (upper = ?3 OR json_extract(CAST(boundary AS TEXT), '$.start') <= ?3) ORDER BY upper ASC LIMIT 1")
                .bind(&self.namespace).bind(id.to_string()).bind(raw(upper)?).exec(&mut tx).await?,
            None => sql::query("SELECT upper, boundary FROM cache_pages WHERE namespace = ?1 AND session = ?2 ORDER BY upper DESC LIMIT 1")
                .bind(&self.namespace).bind(id.to_string()).exec(&mut tx).await?,
        };
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let mut fields = columns(row)?;
        let page_upper = positive(take::<i64>(&mut fields)?)?;
        let boundary: Boundary = serde_json::from_slice(&take::<Vec<u8>>(&mut fields)?)?;
        let upper = requested.unwrap_or(frontier);
        if page_upper > frontier {
            return Err(Error::Invalid("page exceeds frontier"));
        }
        let events = sql::query("SELECT cursor, payload FROM cache_events WHERE namespace = ?1 AND session = ?2 AND cursor >= ?3 AND cursor <= ?4 ORDER BY cursor")
            .bind(&self.namespace).bind(id.to_string()).bind(raw(boundary.start)?).bind(raw(upper)?)
            .exec(&mut tx).await?.into_iter().map(|row| {
                let mut fields = columns(row)?;
                let cursor = positive(take::<i64>(&mut fields)?)?;
                let event: Event = serde_json::from_slice(&take::<Vec<u8>>(&mut fields)?)?;
                if event.session != id || event.cursor.raw() != cursor {
                    return Err(Error::Invalid("event key mismatch"));
                }
                Ok(event)
            }).collect::<Result<Vec<_>, Error>>()?;
        let window = HistoryWindow {
            cursor: if requested.is_some() {
                session.meta.revision.raw()
            } else {
                upper
            },
            session,
            events,
            context: boundary.context,
            before: boundary.before,
        };
        validate(&window)?;
        if window.events.last().map_or(0, |event| event.cursor.raw()) != upper {
            return Err(Error::Invalid("missing retained events at page end"));
        }
        if window
            .events
            .first()
            .map(|event| event.cursor.raw())
            .unwrap_or(upper.saturating_add(1))
            != boundary.start
        {
            return Err(Error::Invalid("missing retained events"));
        }
        self.touch(&mut tx, id).await?;
        tx.commit().await?;
        Ok(Some(window))
    }

    pub(super) async fn store(&self, window: &HistoryWindow) -> Result<(), Error> {
        validate(window)?;
        let id = window.session.meta.id;
        // Older responses carry the current snapshot cursor too. Only their
        // actual event range establishes retained history, never that cursor.
        let upper = window.events.last().map_or(0, |event| event.cursor.raw());
        let mut db = self.db.clone();
        let mut tx = transaction(&mut db).await?;
        let existing = self.session(&mut tx, id).await?;
        let frontier = existing
            .as_ref()
            .map_or(upper, |old| old.frontier.max(upper));
        let snapshot = match existing {
            Some(old) if old.snapshot.meta.revision > window.session.meta.revision => old.snapshot,
            _ => window.session.clone(),
        };
        sql::statement("INSERT INTO cache_sessions(namespace, session, snapshot, frontier, touched, bytes) VALUES (?1, ?2, ?3, ?4, 0, 0) ON CONFLICT(namespace, session) DO UPDATE SET snapshot = excluded.snapshot, frontier = excluded.frontier")
            .bind(&self.namespace).bind(id.to_string()).bind(serde_json::to_vec(&snapshot)?).bind(raw(frontier)?)
            .exec(&mut tx).await?;
        for event in &window.events {
            self.insert_event(&mut tx, event).await?;
        }
        let mut boundary = Boundary {
            start: window
                .events
                .first()
                .map_or(upper.saturating_add(1), |event| event.cursor.raw()),
            before: window.before,
            context: window.context.clone(),
        };
        let prior = sql::query(
            "SELECT boundary FROM cache_pages WHERE namespace = ?1 AND session = ?2 AND upper = ?3",
        )
        .bind(&self.namespace)
        .bind(id.to_string())
        .bind(raw(upper)?)
        .exec(&mut tx)
        .await?
        .into_iter()
        .next();
        if let Some(prior) = prior {
            let saved: Boundary = serde_json::from_slice(&take::<Vec<u8>>(&mut columns(prior)?)?)?;
            // A narrower overlapping fetch must not discard the only saved
            // safe boundary for already retained history.
            if saved.start < boundary.start {
                boundary = saved;
            }
        }
        sql::statement("INSERT INTO cache_pages(namespace, session, upper, boundary) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(namespace, session, upper) DO UPDATE SET boundary = excluded.boundary")
            .bind(&self.namespace).bind(id.to_string()).bind(raw(upper)?)
            .bind(serde_json::to_vec(&boundary)?).exec(&mut tx).await?;
        self.account(&mut tx, id).await?;
        self.evict(&mut tx, Some(id)).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn append(&self, id: SessionId, events: &[Event]) -> Result<(), Error> {
        let mut db = self.db.clone();
        let mut tx = transaction(&mut db).await?;
        let Some(SavedSession {
            mut snapshot,
            mut frontier,
        }) = self.session(&mut tx, id).await?
        else {
            return Ok(());
        };
        for event in events {
            if event.cursor.raw() == 0 || event.is_transient() {
                continue;
            }
            if event.session != id {
                return Err(Error::Invalid("append belongs to another session"));
            }
            if event.cursor.raw() <= frontier {
                continue;
            }
            if frontier.checked_add(1) != Some(event.cursor.raw()) {
                break;
            }
            self.insert_event(&mut tx, event).await?;
            snapshot.meta.apply_event(event);
            frontier = event.cursor.raw();
        }
        sql::statement("UPDATE cache_sessions SET snapshot = ?3, frontier = ?4 WHERE namespace = ?1 AND session = ?2")
            .bind(&self.namespace).bind(id.to_string()).bind(serde_json::to_vec(&snapshot)?).bind(raw(frontier)?)
            .exec(&mut tx).await?;
        self.account(&mut tx, id).await?;
        self.evict(&mut tx, Some(id)).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn remove(&self, id: SessionId) -> Result<(), Error> {
        let mut db = self.db.clone();
        let mut tx = transaction(&mut db).await?;
        delete(&mut tx, &self.namespace, &id.to_string()).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn session(
        &self,
        tx: &mut Transaction<'_>,
        id: SessionId,
    ) -> Result<Option<SavedSession>, Error> {
        let row = sql::query(
            "SELECT snapshot, frontier FROM cache_sessions WHERE namespace = ?1 AND session = ?2",
        )
        .bind(&self.namespace)
        .bind(id.to_string())
        .exec(tx)
        .await?
        .into_iter()
        .next();
        row.map(|row| {
            let mut fields = columns(row)?;
            let snapshot: SessionSnapshot = serde_json::from_slice(&take::<Vec<u8>>(&mut fields)?)?;
            let frontier = positive(take::<i64>(&mut fields)?)?;
            if snapshot.meta.id != id || snapshot.meta.revision.raw() < frontier {
                return Err(Error::Invalid("snapshot key or revision mismatch"));
            }
            Ok(SavedSession { snapshot, frontier })
        })
        .transpose()
    }

    async fn insert_event(&self, tx: &mut Transaction<'_>, event: &Event) -> Result<(), Error> {
        sql::statement("INSERT INTO cache_events(namespace, session, cursor, payload) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(namespace, session, cursor) DO NOTHING")
            .bind(&self.namespace).bind(event.session.to_string()).bind(raw(event.cursor.raw())?)
            .bind(serde_json::to_vec(event)?).exec(tx).await?;
        Ok(())
    }

    async fn touch(&self, tx: &mut Transaction<'_>, id: SessionId) -> Result<(), Error> {
        sql::statement("UPDATE cache_sessions SET touched = (SELECT COALESCE(MAX(touched), 0) + 1 FROM cache_sessions) WHERE namespace = ?1 AND session = ?2")
            .bind(&self.namespace).bind(id.to_string()).exec(tx).await?;
        Ok(())
    }

    async fn account(&self, tx: &mut Transaction<'_>, id: SessionId) -> Result<(), Error> {
        sql::statement("UPDATE cache_sessions SET bytes = length(snapshot) + ?3 + (SELECT COALESCE(SUM(length(payload) + ?3), 0) FROM cache_events WHERE namespace = ?1 AND session = ?2) + (SELECT COALESCE(SUM(length(boundary) + ?3), 0) FROM cache_pages WHERE namespace = ?1 AND session = ?2) WHERE namespace = ?1 AND session = ?2")
            .bind(&self.namespace).bind(id.to_string()).bind(ROW_OVERHEAD).exec(&mut *tx).await?;
        self.touch(tx, id).await
    }

    async fn evict(
        &self,
        tx: &mut Transaction<'_>,
        written: Option<SessionId>,
    ) -> Result<(), Error> {
        if let Some(id) = written {
            let bytes = scalar(
                sql::query(
                    "SELECT bytes FROM cache_sessions WHERE namespace = ?1 AND session = ?2",
                )
                .bind(&self.namespace)
                .bind(id.to_string())
                .exec(&mut *tx)
                .await?,
            )?;
            if bytes > self.limit {
                delete(tx, &self.namespace, &id.to_string()).await?;
            }
        }
        loop {
            let total = scalar(
                sql::query("SELECT COALESCE(SUM(bytes), 0) FROM cache_sessions")
                    .exec(&mut *tx)
                    .await?,
            )?;
            if total <= self.limit {
                break;
            }
            let row = sql::query("SELECT namespace, session FROM cache_sessions ORDER BY touched, namespace, session LIMIT 1")
                .exec(&mut *tx).await?.into_iter().next().ok_or(Error::Invalid("missing eviction victim"))?;
            let mut fields = columns(row)?;
            let namespace: String = take(&mut fields)?;
            let session: String = take(&mut fields)?;
            delete(tx, &namespace, &session).await?;
        }
        Ok(())
    }
}

async fn transaction(db: &mut toasty::Db) -> Result<Transaction<'_>, Error> {
    let mut tx = db.transaction().await?;
    // Toasty's SQLite driver begins deferred transactions. Acquire the writer
    // reservation before reads to avoid lock-upgrade races between GUI windows.
    sql::statement("UPDATE cache_sessions SET touched = touched WHERE 0")
        .exec(&mut tx)
        .await?;
    Ok(tx)
}

async fn delete(tx: &mut Transaction<'_>, namespace: &str, session: &str) -> Result<(), Error> {
    for table in ["cache_events", "cache_pages", "cache_sessions"] {
        sql::statement(format!(
            "DELETE FROM {table} WHERE namespace = ?1 AND session = ?2"
        ))
        .bind(namespace)
        .bind(session)
        .exec(&mut *tx)
        .await?;
    }
    Ok(())
}

fn namespace(options: &ConnectionOptions) -> Result<String, Error> {
    // Hash the enum discriminant and non-lossy path debug representation too:
    // display alone is not injective for non-UTF8 Unix socket paths.
    // Event-count pages cannot supply the whole-turn boundaries used by this reader.
    let key = serde_json::to_vec(&(
        "user-turn-windows-v1",
        format!("{:?}", options.target),
        &options.tls,
    ))?;
    Ok(format!("{:x}", Sha256::digest(key)))
}

fn validate(window: &HistoryWindow) -> Result<(), Error> {
    raw(window.cursor)?;
    if window.cursor > window.session.meta.revision.raw() {
        return Err(Error::Invalid("window exceeds metadata revision"));
    }
    let mut previous = None;
    for event in &window.events {
        if event.session != window.session.meta.id
            || event.cursor.raw() == 0
            || event.is_transient()
            || previous.is_some_and(|cursor: u64| cursor.checked_add(1) != Some(event.cursor.raw()))
        {
            return Err(Error::Invalid(
                "window is not contiguous replayable history",
            ));
        }
        previous = Some(event.cursor.raw());
    }
    if previous.is_some_and(|last| last > window.cursor) {
        return Err(Error::Invalid("window exceeds its snapshot"));
    }
    let start = window.events.first().map_or(1, |event| event.cursor.raw());
    if window
        .before
        .is_some_and(|before| before != start || before == 0)
        || window.context.iter().any(|event| {
            event.session != window.session.meta.id
                || event.cursor.raw() == 0
                || event.cursor.raw() >= start
                || event.is_transient()
        })
    {
        return Err(Error::Invalid("invalid context or older-page boundary"));
    }
    Ok(())
}

fn raw(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::Invalid("cursor exceeds SQLite range"))
}

fn positive(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::Invalid("negative cursor"))
}

fn columns(row: Value) -> Result<std::vec::IntoIter<Value>, Error> {
    match row {
        Value::Record(fields) => Ok(fields.into_iter().collect::<Vec<_>>().into_iter()),
        _ => Err(Error::Invalid("invalid database row")),
    }
}

fn take<T: TryFrom<Value>>(fields: &mut std::vec::IntoIter<Value>) -> Result<T, Error> {
    fields
        .next()
        .and_then(|field| T::try_from(field).ok())
        .ok_or(Error::Invalid("invalid database column"))
}

fn scalar(rows: Vec<Value>) -> Result<i64, Error> {
    take(&mut columns(
        rows.into_iter()
            .next()
            .ok_or(Error::Invalid("missing database row"))?,
    )?)
}
