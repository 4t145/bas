use std::num::NonZeroUsize;

use super::localdb::TestSql;
use super::{Error, LocalDb, Memory, PromptAcceptance, ResumeData, Storage};
use bas::model::{Cursor, Event, EventKind, Owner, SessionId, SessionMeta, TurnId, TurnReceipt};

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

fn accepted_event(state: &mut SessionMeta) -> (Event, PromptAcceptance) {
    let turn = TurnId::new();
    let event = Event::new(
        state.id,
        state.revision.next(),
        EventKind::TurnStarted { turn },
    );
    assert!(state.apply_event(&event));
    let acceptance = PromptAcceptance {
        receipt: TurnReceipt {
            turn,
            cursor: event.cursor,
        },
        fingerprint: [42; 32],
    };
    (event, acceptance)
}

async fn acceptance_contract(storage: &dyn Storage) {
    let mut state = meta();
    let id = state.id;
    let unknown = TurnId::new();
    assert!(matches!(
        storage.acceptance(id, unknown).await,
        Err(Error::NotFound(_))
    ));
    storage.create(state.clone(), None).await.expect("create");
    assert_eq!(storage.acceptance(id, unknown).await.expect("query"), None);
    let (event, acceptance) = accepted_event(&mut state);
    let mut invalid = acceptance;
    invalid.receipt.cursor = Cursor::START;
    assert!(
        storage
            .commit(state.clone(), std::slice::from_ref(&event), Some(invalid))
            .await
            .is_err()
    );
    invalid = acceptance;
    invalid.receipt.turn = unknown;
    assert!(
        storage
            .commit(state.clone(), std::slice::from_ref(&event), Some(invalid))
            .await
            .is_err()
    );
    assert!(
        storage
            .commit(state.clone(), &[], Some(acceptance))
            .await
            .is_err()
    );
    assert_eq!(
        storage
            .acceptance(id, acceptance.receipt.turn)
            .await
            .expect("query"),
        None
    );
    assert!(
        storage
            .page(id, None, 8)
            .await
            .expect("unchanged history")
            .events
            .is_empty()
    );
    storage
        .commit(
            state.clone(),
            std::slice::from_ref(&event),
            Some(acceptance),
        )
        .await
        .expect("accept");
    for fingerprint in [acceptance.fingerprint, [7; 32]] {
        let mut duplicate = acceptance;
        duplicate.fingerprint = fingerprint;
        let mut changed = state.clone();
        let batch = events(&mut changed, 1);
        assert!(matches!(
            storage.commit(changed, &batch, Some(duplicate)).await,
            Err(Error::SubmissionExists(turn)) if turn == acceptance.receipt.turn
        ));
        assert_eq!(
            storage.load(id).await.expect("load").expect("exists").meta,
            state
        );
        assert_eq!(
            storage.page(id, None, 8).await.expect("page").events,
            vec![event.clone()]
        );
        assert_eq!(
            storage
                .acceptance(id, acceptance.receipt.turn)
                .await
                .expect("query"),
            Some(acceptance)
        );
    }
    let batch = events(&mut state, 5);
    storage
        .commit(state.clone(), &batch, None)
        .await
        .expect("later events");
    // In Memory the original event has been evicted; in SQLite this page omits it.
    storage
        .page(id, Some(batch[2].cursor), 1)
        .await
        .expect("later page");
    assert_eq!(
        storage
            .acceptance(id, acceptance.receipt.turn)
            .await
            .expect("query"),
        Some(acceptance)
    );
    storage.delete(id).await.expect("delete");
    assert!(matches!(
        storage.acceptance(id, acceptance.receipt.turn).await,
        Err(Error::NotFound(_))
    ));
    let mut recreated = state;
    recreated.revision = Cursor::START;
    storage
        .create(recreated, None)
        .await
        .expect("recreate same id");
    assert_eq!(
        storage
            .acceptance(id, acceptance.receipt.turn)
            .await
            .expect("query"),
        None
    );
}

#[tokio::test]
async fn acceptance_backends_share_contract() {
    acceptance_contract(&Memory::new(NonZeroUsize::new(2).expect("nonzero"))).await;
    acceptance_contract(&LocalDb::open(":memory:").await.expect("open")).await;
}

#[tokio::test]
async fn acceptance_rolls_back_with_failed_sqlite_batch() {
    let db = LocalDb::open(":memory:").await.expect("open");
    let mut state = meta();
    let original = state.clone();
    db.create(state.clone(), None).await.expect("create");
    let (event, acceptance) = accepted_event(&mut state);
    let mut batch = vec![event];
    batch.extend(events(&mut state, 2));
    db.test_sql(TestSql::RejectThirdEvent)
        .await
        .expect("trigger");
    assert!(db.commit(state, &batch, Some(acceptance)).await.is_err());
    assert_eq!(
        db.load(original.id)
            .await
            .expect("load")
            .expect("exists")
            .meta,
        original
    );
    assert!(
        db.page(original.id, None, 8)
            .await
            .expect("page")
            .events
            .is_empty()
    );
    assert_eq!(
        db.acceptance(original.id, acceptance.receipt.turn)
            .await
            .expect("query"),
        None
    );
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
    storage
        .commit(state.clone(), &batch, None)
        .await
        .expect("commit");
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
        storage.commit(invalid, &[], None).await,
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
    memory
        .commit(state.clone(), &batch, None)
        .await
        .expect("commit");
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
        db.commit(state.clone(), &batch, None)
            .await
            .expect("commit");
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
