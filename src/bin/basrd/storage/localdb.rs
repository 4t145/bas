use std::path::Path;

use basr::model::{Cursor, Event, SessionId, SessionMeta, TurnId, TurnReceipt};
use toasty::{
    migration::{MigrationFile, MigrationSet},
    sql,
    stmt::Value,
};

use super::{
    Error, Page, PromptAcceptance, ResumeData, StoredSession, check_page, validate_acceptance,
    validate_commit, validate_create,
};

#[path = "localdb/worker.rs"]
mod worker;
pub use worker::LocalDb;
#[cfg(test)]
pub(super) use worker::TestSql;

// Indexed lookup fields are separate from the opaque JSON snapshots. The
// versioned migration, not push_schema, defines the production schema.
#[derive(Debug, toasty::Model)]
#[table = "sessions"]
struct SessionRow {
    #[key]
    id: String,
    metadata: Vec<u8>,
    updated_at: i64,
    revision: i64,
    resume_version: Option<i64>,
    resume_payload: Option<Vec<u8>>,
}

#[derive(Debug, toasty::Model)]
#[table = "events"]
#[key(partition = session_id, local = cursor)]
struct EventRow {
    session_id: String,
    cursor: i64,
    payload: Vec<u8>,
}

#[derive(Debug, toasty::Model)]
#[table = "accepted_prompts"]
#[key(partition = session_id, local = turn_id)]
struct AcceptanceRow {
    session_id: String,
    turn_id: String,
    cursor: i64,
    fingerprint: Vec<u8>,
}

const MIGRATIONS: MigrationSet = MigrationSet::new(&[
    MigrationFile::new(
        1,
        "session_history",
        include_str!("migrations/0001_session_history.sql"),
    ),
    MigrationFile::new(
        2,
        "accepted_prompts",
        include_str!("migrations/0002_accepted_prompts.sql"),
    ),
]);

/// Runs only on the dedicated database runtime.
struct SqliteStore {
    db: toasty::Db,
}

impl SqliteStore {
    /// Opens a SQLite database and applies pending migrations.
    ///
    /// # Errors
    /// Returns I/O, connection or migration errors; refuses insecure Unix paths.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            secure_path(path)?;
        }
        let driver = if path == Path::new(":memory:") {
            toasty_driver_sqlite::Sqlite::in_memory()
        } else {
            toasty_driver_sqlite::Sqlite::open(path)
        };
        let db = toasty::Db::builder()
            .models(toasty::models!(SessionRow, EventRow, AcceptanceRow))
            .max_pool_size(1)
            .build(driver)
            .await?;
        verify_existing_schema(&db).await?;
        MIGRATIONS.apply(&db).await?;
        Ok(Self { db })
    }
}

async fn verify_existing_schema(db: &toasty::Db) -> Result<(), Error> {
    let version = sql::query("PRAGMA user_version")
        .exec(&mut db.clone())
        .await?
        .into_iter()
        .next()
        .ok_or(Error::Corrupt("missing schema version"))?;
    let version: i64 = field(&mut columns(version)?)?;
    if !(0..=2).contains(&version) {
        return Err(Error::InvalidCommit("unsupported SQLite schema version"));
    }
    let tables = sql::query(
        "SELECT name FROM sqlite_master WHERE name IN ('sessions', 'events', 'sessions_recent', 'accepted_prompts')",
    )
    .exec(&mut db.clone())
    .await?;
    if (version == 0 && !tables.is_empty())
        || (version == 1 && tables.len() != 3)
        || (version == 2 && tables.len() != 4)
    {
        return Err(Error::Corrupt(
            "existing session schema is incomplete or unversioned",
        ));
    }
    if version >= 1 {
        let session_rows = sql::query("PRAGMA table_info(sessions)")
            .exec(&mut db.clone())
            .await?;
        let mut session_columns = std::collections::HashMap::new();
        for row in session_rows {
            let fields = columns(row)?;
            let name = String::try_from(fields[1].clone())
                .map_err(|_| Error::Corrupt("invalid sessions column name"))?;
            let key = i64::try_from(fields[5].clone())
                .map_err(|_| Error::Corrupt("invalid sessions primary key"))?;
            session_columns.insert(name, key);
        }
        if session_columns.get("id") != Some(&1)
            || [
                "metadata",
                "updated_at",
                "revision",
                "resume_version",
                "resume_payload",
            ]
            .iter()
            .any(|name| !session_columns.contains_key(*name))
        {
            return Err(Error::Corrupt("incompatible sessions table"));
        }
        let rows = sql::query("PRAGMA table_info(events)")
            .exec(&mut db.clone())
            .await?;
        let mut session_key = false;
        let mut cursor_key = false;
        let mut payload = false;
        for row in rows {
            let fields = columns(row)?;
            let name = String::try_from(fields[1].clone())
                .map_err(|_| Error::Corrupt("invalid events column name"))?;
            let key = i64::try_from(fields[5].clone())
                .map_err(|_| Error::Corrupt("invalid events primary key"))?;
            match name.as_str() {
                "session_id" => session_key = key == 1,
                "cursor" => cursor_key = key == 2,
                "payload" => payload = true,
                _ => {}
            }
        }
        if !session_key || !cursor_key || !payload {
            return Err(Error::Corrupt(
                "events must have the (session_id, cursor) primary key",
            ));
        }
    }
    if version == 2 {
        let rows = sql::query("PRAGMA table_info(accepted_prompts)")
            .exec(&mut db.clone())
            .await?;
        let mut actual = std::collections::HashMap::new();
        for row in rows {
            let fields = columns(row)?;
            let name = String::try_from(fields[1].clone())
                .map_err(|_| Error::Corrupt("acceptance column"))?;
            let kind = String::try_from(fields[2].clone())
                .map_err(|_| Error::Corrupt("acceptance type"))?;
            let required = i64::try_from(fields[3].clone())
                .map_err(|_| Error::Corrupt("acceptance nullability"))?;
            let key =
                i64::try_from(fields[5].clone()).map_err(|_| Error::Corrupt("acceptance key"))?;
            actual.insert(name, (kind.to_ascii_uppercase(), required, key));
        }
        if actual.len() != 4 {
            return Err(Error::Corrupt("incompatible accepted_prompts columns"));
        }
        for (name, kind, key) in [
            ("session_id", "TEXT", 1),
            ("turn_id", "TEXT", 2),
            ("cursor", "INTEGER", 0),
            ("fingerprint", "BLOB", 0),
        ] {
            if actual.get(name) != Some(&(kind.into(), 1, key)) {
                return Err(Error::Corrupt("incompatible accepted_prompts table"));
            }
        }
    }
    Ok(())
}

fn secure_path(path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let info = std::fs::symlink_metadata(parent)?;
            if !info.is_dir() || info.permissions().mode() & 0o077 != 0 {
                return Err(private_path_error("database directory must be private"));
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)?;
        let info = std::fs::symlink_metadata(path)?;
        if !info.is_file() || info.permissions().mode() & 0o077 != 0 {
            return Err(private_path_error("database file must be private"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn private_path_error(message: &'static str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message,
    ))
}

fn columns(row: Value) -> Result<Vec<Value>, Error> {
    match row {
        Value::Record(fields) => Ok(fields.into_iter().collect()),
        _ => Err(Error::Corrupt("invalid database row")),
    }
}

fn field<T: TryFrom<Value>>(row: &mut Vec<Value>) -> Result<T, Error> {
    T::try_from(row.remove(0)).map_err(|_| Error::Corrupt("invalid database column"))
}

fn revision(raw: i64) -> Result<Cursor, Error> {
    Ok(Cursor::from_raw(
        u64::try_from(raw).map_err(|_| Error::Corrupt("negative revision"))?,
    ))
}

fn raw(cursor: Cursor) -> Result<i64, Error> {
    i64::try_from(cursor.raw()).map_err(|_| Error::CursorOverflow)
}

fn decode_session(row: Value) -> Result<StoredSession, Error> {
    let mut fields = columns(row)?;
    let id: String = field(&mut fields)?;
    let rev: i64 = field(&mut fields)?;
    let data: Vec<u8> = field(&mut fields)?;
    let resume = match (fields.remove(0), fields.remove(0)) {
        (Value::Null, Value::Null) => None,
        (version, payload) => Some(ResumeData {
            version: u32::try_from(
                i64::try_from(version).map_err(|_| Error::Corrupt("resume version"))?,
            )
            .map_err(|_| Error::Corrupt("resume version"))?,
            payload: Vec::<u8>::try_from(payload).map_err(|_| Error::Corrupt("resume payload"))?,
        }),
    };
    let meta: SessionMeta = serde_json::from_slice(&data)?;
    if meta.id.to_string() != id || meta.revision != revision(rev)? {
        return Err(Error::Corrupt("session metadata does not match its row"));
    }
    Ok(StoredSession { meta, resume })
}

fn decode_event(id: SessionId, latest: Cursor, row: Value) -> Result<Event, Error> {
    let mut fields = columns(row)?;
    let cursor: i64 = field(&mut fields)?;
    let payload: Vec<u8> = field(&mut fields)?;
    let event: Event = serde_json::from_slice(&payload)?;
    if event.session != id || event.cursor != revision(cursor)? || event.cursor > latest {
        return Err(Error::Corrupt("event payload does not match its row"));
    }
    Ok(event)
}

fn resume_values(resume: Option<ResumeData>) -> (Value, Value) {
    resume.map_or((Value::Null, Value::Null), |r| {
        (Value::I64(i64::from(r.version)), Value::Bytes(r.payload))
    })
}

impl SqliteStore {
    async fn create(&self, meta: SessionMeta, resume: Option<ResumeData>) -> Result<(), Error> {
        validate_create(&meta)?;
        if self.load(meta.id).await?.is_some() {
            return Err(Error::AlreadyExists(meta.id));
        }
        let (version, payload) = resume_values(resume);
        let inserted = sql::statement("INSERT INTO sessions(id, metadata, updated_at, revision, resume_version, resume_payload) VALUES (?1, ?2, ?3, 0, ?4, ?5)")
            .bind(meta.id.to_string()).bind(serde_json::to_vec(&meta)?)
            .bind(meta.updated_at.millis())
            .bind_typed(version, toasty::schema::db::Type::Integer(8))
            .bind_typed(payload, toasty::schema::db::Type::Blob)
            .exec(&mut self.db.clone()).await;
        if let Err(error) = inserted {
            // Another writer can win between the initial lookup and INSERT.
            if self.load(meta.id).await?.is_some() {
                return Err(Error::AlreadyExists(meta.id));
            }
            return Err(error.into());
        }
        Ok(())
    }

    async fn commit(
        &self,
        meta: SessionMeta,
        events: &[Event],
        acceptance: Option<PromptAcceptance>,
    ) -> Result<(), Error> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await?;
        let previous = sql::query("SELECT id, revision, metadata, resume_version, resume_payload FROM sessions WHERE id = ?1")
            .bind(meta.id.to_string()).exec(&mut tx).await?
            .into_iter().next().ok_or(Error::NotFound(meta.id))?;
        if let Some(acceptance) = acceptance {
            let existing = sql::query(
                "SELECT turn_id FROM accepted_prompts WHERE session_id = ?1 AND turn_id = ?2",
            )
            .bind(meta.id.to_string())
            .bind(acceptance.receipt.turn.to_string())
            .exec(&mut tx)
            .await?;
            if !existing.is_empty() {
                return Err(Error::SubmissionExists(acceptance.receipt.turn));
            }
            validate_acceptance(events, acceptance)?;
        }
        validate_commit(&decode_session(previous)?.meta, &meta, events)?;
        if let Some(acceptance) = acceptance {
            sql::statement("INSERT INTO accepted_prompts(session_id, turn_id, cursor, fingerprint) VALUES (?1, ?2, ?3, ?4)")
                .bind(meta.id.to_string()).bind(acceptance.receipt.turn.to_string())
                .bind(raw(acceptance.receipt.cursor)?).bind(acceptance.fingerprint.to_vec())
                .exec(&mut tx).await?;
        }
        for event in events {
            sql::statement("INSERT INTO events(session_id, cursor, payload) VALUES (?1, ?2, ?3)")
                .bind(meta.id.to_string())
                .bind(raw(event.cursor)?)
                .bind(serde_json::to_vec(event)?)
                .exec(&mut tx)
                .await?;
        }
        sql::statement(
            "UPDATE sessions SET metadata = ?1, updated_at = ?2, revision = ?3 WHERE id = ?4",
        )
        .bind(serde_json::to_vec(&meta)?)
        .bind(meta.updated_at.millis())
        .bind(raw(meta.revision)?)
        .bind(meta.id.to_string())
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn acceptance(
        &self,
        session: SessionId,
        turn: TurnId,
    ) -> Result<Option<PromptAcceptance>, Error> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await?;
        let row = sql::query("SELECT id, revision, metadata, resume_version, resume_payload FROM sessions WHERE id = ?1")
            .bind(session.to_string()).exec(&mut tx).await?
            .into_iter().next().ok_or(Error::NotFound(session))?;
        let latest = decode_session(row)?.meta.revision;
        let row = sql::query("SELECT cursor, fingerprint FROM accepted_prompts WHERE session_id = ?1 AND turn_id = ?2")
            .bind(session.to_string()).bind(turn.to_string()).exec(&mut tx).await?
            .into_iter().next();
        let Some(row) = row else { return Ok(None) };
        let mut fields = columns(row)?;
        let cursor = revision(field(&mut fields)?)?;
        let fingerprint: Vec<u8> = field(&mut fields)?;
        let fingerprint = fingerprint
            .try_into()
            .map_err(|_| Error::Corrupt("acceptance fingerprint length"))?;
        let event =
            sql::query("SELECT cursor, payload FROM events WHERE session_id = ?1 AND cursor = ?2")
                .bind(session.to_string())
                .bind(raw(cursor)?)
                .exec(&mut tx)
                .await?
                .into_iter()
                .next()
                .ok_or(Error::Corrupt("acceptance event missing"))?;
        let event = decode_event(session, latest, event)?;
        let acceptance = PromptAcceptance {
            receipt: TurnReceipt { turn, cursor },
            fingerprint,
        };
        validate_acceptance(&[event], acceptance)
            .map_err(|_| Error::Corrupt("acceptance event mismatch"))?;
        Ok(Some(acceptance))
    }

    async fn save_resume(&self, id: SessionId, resume: Option<ResumeData>) -> Result<(), Error> {
        let (version, payload) = resume_values(resume);
        let count = sql::statement(
            "UPDATE sessions SET resume_version = ?1, resume_payload = ?2 WHERE id = ?3",
        )
        .bind_typed(version, toasty::schema::db::Type::Integer(8))
        .bind_typed(payload, toasty::schema::db::Type::Blob)
        .bind(id.to_string())
        .exec(&mut self.db.clone())
        .await?;
        if count == 0 {
            return Err(Error::NotFound(id));
        }
        Ok(())
    }

    async fn load(&self, id: SessionId) -> Result<Option<StoredSession>, Error> {
        sql::query("SELECT id, revision, metadata, resume_version, resume_payload FROM sessions WHERE id = ?1")
            .bind(id.to_string()).exec(&mut self.db.clone()).await?
            .into_iter().next().map(decode_session).transpose()
    }

    async fn list(&self) -> Result<Vec<SessionMeta>, Error> {
        sql::query("SELECT id, revision, metadata, resume_version, resume_payload FROM sessions ORDER BY updated_at DESC, id DESC")
            .exec(&mut self.db.clone()).await?.into_iter()
            .map(|row| Ok(decode_session(row)?.meta)).collect()
    }

    async fn page(
        &self,
        id: SessionId,
        after: Option<Cursor>,
        limit: usize,
    ) -> Result<Page, Error> {
        let latest = self
            .load(id)
            .await?
            .ok_or(Error::NotFound(id))?
            .meta
            .revision;
        check_page(after, limit, latest)?;
        let rows = sql::query("SELECT cursor, payload FROM events WHERE session_id = ?1 AND cursor > ?2 AND cursor <= ?3 ORDER BY cursor ASC LIMIT ?4")
            .bind(id.to_string()).bind(raw(after.unwrap_or(Cursor::START))?)
            .bind(raw(latest)?)
            .bind(i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX))
            .exec(&mut self.db.clone()).await?;
        let mut events = rows
            .into_iter()
            .map(|row| decode_event(id, latest, row))
            .collect::<Result<Vec<_>, _>>()?;
        let next = if events.len() > limit {
            events.truncate(limit);
            events.last().map(|event| event.cursor)
        } else {
            None
        };
        Ok(Page { events, next })
    }

    async fn tail(&self, id: SessionId, count: usize) -> Result<Vec<Event>, Error> {
        let latest = self
            .load(id)
            .await?
            .ok_or(Error::NotFound(id))?
            .meta
            .revision;
        if count == 0 {
            return Ok(vec![]);
        }
        let rows = sql::query("SELECT cursor, payload FROM events WHERE session_id = ?1 AND cursor <= ?2 ORDER BY cursor DESC LIMIT ?3")
            .bind(id.to_string()).bind(raw(latest)?)
            .bind(i64::try_from(count).unwrap_or(i64::MAX))
            .exec(&mut self.db.clone()).await?;
        let mut events = rows
            .into_iter()
            .map(|row| decode_event(id, latest, row))
            .collect::<Result<Vec<_>, _>>()?;
        events.reverse();
        Ok(events)
    }

    async fn delete(&self, id: SessionId) -> Result<bool, Error> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await?;
        let deleted = sql::statement("DELETE FROM sessions WHERE id = ?1")
            .bind(id.to_string())
            .exec(&mut tx)
            .await?
            != 0;
        sql::statement("DELETE FROM events WHERE session_id = ?1")
            .bind(id.to_string())
            .exec(&mut tx)
            .await?;
        sql::statement("DELETE FROM accepted_prompts WHERE session_id = ?1")
            .bind(id.to_string())
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
        Ok(deleted)
    }
}
