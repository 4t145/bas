use std::num::NonZeroUsize;

use super::localdb::TestSql;
use super::{Error, LocalDb, Memory, ResumeData, Storage};
use basr::model::{Cursor, Event, EventKind, Owner, SessionId, SessionMeta};

fn meta() -> SessionMeta {
    SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current())
}

fn events(meta: &mut SessionMeta, count: usize) -> Vec<Event> {
    (0..count)
        .map(|_| {
            let event = Event::new(
                meta.id,
                meta.revision.next(),
                EventKind::TitleChanged {
                    title: "title".into(),
                },
            );
            assert!(meta.apply_event(&event));
            event
        })
        .collect()
}

async fn contract(storage: &dyn Storage) {
    let mut state = meta();
    let id = state.id;
    storage
        .create(state.clone(), Some(ResumeData::acp("private")))
        .await
        .expect("create");
    assert!(matches!(
        storage.create(state.clone(), None).await,
        Err(Error::AlreadyExists(_))
    ));
    let batch = events(&mut state, 3);
    storage.commit(state.clone(), &batch).await.expect("commit");
    assert_eq!(
        storage.page(id, None, 2).await.expect("page").next,
        Some(batch[1].cursor)
    );
    assert_eq!(
        storage
            .page(id, Some(batch[1].cursor), 2)
            .await
            .expect("page")
            .events,
        batch[2..]
    );
    assert_eq!(storage.tail(id, 2).await.expect("tail"), batch[1..]);
    assert_eq!(
        storage.load(id).await.expect("load").expect("exists").meta,
        state
    );
    assert_eq!(storage.list().await.expect("list"), vec![state.clone()]);
    storage.save_resume(id, None).await.expect("clear resume");
    assert_eq!(
        storage
            .load(id)
            .await
            .expect("load")
            .expect("exists")
            .resume,
        None
    );
    let mut invalid = state.clone();
    invalid.revision = invalid.revision.next();
    assert!(matches!(
        storage.commit(invalid, &[]).await,
        Err(Error::InvalidCommit(_))
    ));
    assert!(storage.delete(id).await.expect("delete"));
    assert!(storage.page(id, None, 1).await.is_err());
}

#[tokio::test]
async fn backends_share_contract() {
    let memory = Memory::new(NonZeroUsize::new(8).expect("nonzero"));
    assert!(!memory.is_persistent());
    contract(&memory).await;
    let db = LocalDb::open(":memory:").await.expect("open");
    assert!(db.is_persistent());
    contract(&db).await;
}

#[tokio::test]
async fn memory_eviction_keeps_snapshot_and_resume() {
    let memory = Memory::new(NonZeroUsize::new(2).expect("nonzero"));
    let mut state = meta();
    memory
        .create(state.clone(), Some(ResumeData::acp("private")))
        .await
        .expect("create");
    let batch = events(&mut state, 4);
    memory.commit(state.clone(), &batch).await.expect("commit");
    assert!(matches!(
        memory.page(state.id, Some(Cursor::START), 2).await,
        Err(Error::Expired { .. })
    ));
    assert_eq!(
        memory.page(state.id, None, 8).await.expect("page").events,
        batch[2..]
    );
    assert_eq!(
        memory
            .load(state.id)
            .await
            .expect("load")
            .expect("exists")
            .meta,
        state
    );
}

#[tokio::test]
async fn sqlite_reopens_and_skips_applied_migration() {
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("nested/history.sqlite");
    let mut state = meta();
    let id = state.id;
    {
        let db = LocalDb::open(&path).await.expect("migrate");
        db.create(state.clone(), Some(ResumeData::acp("secret")))
            .await
            .expect("create");
        let batch = events(&mut state, 4);
        db.commit(state.clone(), &batch).await.expect("commit");
    }
    let db = LocalDb::open(&path).await.expect("skip migration");
    assert_eq!(
        db.load(id).await.expect("load").expect("exists").meta,
        state
    );
    assert_eq!(
        db.page(id, Some(Cursor::START), 20)
            .await
            .expect("page")
            .events
            .len(),
        4
    );
    assert_eq!(
        db.load(id)
            .await
            .expect("load")
            .expect("exists")
            .resume
            .expect("resume")
            .acp_session_id()
            .expect("utf8"),
        "secret"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).expect("stat").permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn sqlite_adopts_previous_version_one_schema() {
    let dir = tempfile::tempdir().expect("directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private database directory");
    }
    let path = dir.path().join("history.sqlite");
    let state = meta();
    {
        let db = LocalDb::open(&path).await.expect("create schema");
        db.create(state.clone(), None).await.expect("create");
        // The previous rusqlite adapter had user_version=1 but did not have
        // Toasty's migration history.
        db.test_sql(TestSql::DropMigrationHistory)
            .await
            .expect("simulate legacy schema");
    }
    let db = LocalDb::open(&path)
        .await
        .expect("adopt version one schema");
    assert_eq!(
        db.load(state.id).await.expect("load").expect("exists").meta,
        state
    );
}

#[cfg(unix)]
#[tokio::test]
async fn sqlite_rejects_insecure_path_and_symlink() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().expect("directory");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private directory");
    let path = dir.path().join("history.sqlite");
    let db = LocalDb::open(&path).await.expect("open");
    drop(db);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("make file insecure");
    assert!(matches!(LocalDb::open(&path).await, Err(Error::Io(_))));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("restore permissions");
    let link = dir.path().join("link.sqlite");
    symlink(&path, &link).expect("symlink");
    assert!(matches!(LocalDb::open(&link).await, Err(Error::Io(_))));
}

#[tokio::test]
async fn composite_key_and_failed_batch_rollback() {
    let db = LocalDb::open(":memory:").await.expect("open");
    let mut state = meta();
    let id = state.id;
    db.create(state.clone(), None).await.expect("create");
    let first = events(&mut state, 1);
    db.commit(state.clone(), &first).await.expect("commit");
    let duplicate = db
        .test_sql(TestSql::InsertDuplicate {
            session: id,
            cursor: first[0].cursor,
            payload: serde_json::to_vec(&first[0]).expect("json"),
        })
        .await;
    assert!(duplicate.is_err(), "(session_id,cursor) must be unique");
    let batch = events(&mut state, 2);
    db.test_sql(TestSql::RejectThirdEvent)
        .await
        .expect("trigger");
    assert!(db.commit(state, &batch).await.is_err());
    assert_eq!(
        db.load(id)
            .await
            .expect("load")
            .expect("exists")
            .meta
            .revision,
        first[0].cursor
    );
    assert_eq!(db.page(id, None, 10).await.expect("page").events, first);
}

#[tokio::test(flavor = "current_thread")]
async fn sqlite_lock_wait_does_not_stall_the_daemon_runtime() {
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().expect("directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private database directory");
    }
    let path = dir.path().join("history.sqlite");
    let db = std::sync::Arc::new(LocalDb::open(&path).await.expect("open database"));
    let state = meta();
    db.create(state.clone(), None)
        .await
        .expect("create session");

    let (locked, ready) = tokio::sync::oneshot::channel();
    let blocker = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(&path).expect("external SQLite connection");
        connection
            .execute_batch("BEGIN EXCLUSIVE")
            .expect("hold exclusive database lock");
        let _ = locked.send(());
        std::thread::sleep(Duration::from_millis(350));
        connection.execute_batch("COMMIT").expect("release lock");
    });
    ready.await.expect("external writer acquired its lock");
    let writer = {
        let db = std::sync::Arc::clone(&db);
        tokio::spawn(async move {
            db.save_resume(state.id, Some(ResumeData::acp("locked")))
                .await
        })
    };
    tokio::task::yield_now().await;
    let started = Instant::now();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "SQLite lock wait must not stall the daemon's Tokio worker"
    );
    blocker.join().expect("external writer thread");
    tokio::time::timeout(Duration::from_secs(3), writer)
        .await
        .expect("SQLite write completes")
        .expect("storage task did not panic")
        .expect("storage resumes after lock release");
}

#[cfg(unix)]
#[tokio::test]
async fn sqlite_refuses_legacy_schema_without_the_event_composite_key() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("directory");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private database directory");
    let path = dir.path().join("legacy.sqlite");
    let connection = rusqlite::Connection::open(&path).expect("legacy database");
    connection
        .execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY, metadata BLOB NOT NULL, updated_at INTEGER NOT NULL,
                revision INTEGER NOT NULL, resume_version INTEGER, resume_payload BLOB
            );
            CREATE INDEX sessions_recent ON sessions(updated_at DESC, id DESC);
            CREATE TABLE events (session_id TEXT, cursor INTEGER, payload BLOB);
            PRAGMA user_version = 1;",
        )
        .expect("malformed legacy schema");
    drop(connection);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("private legacy database");
    assert!(matches!(LocalDb::open(&path).await, Err(Error::Corrupt(_))));
}
