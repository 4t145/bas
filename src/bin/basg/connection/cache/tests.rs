use super::*;
use bas::model::{Cursor, EventKind, Owner, SessionMeta};

fn temporary_cache() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temporary cache");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private test fixture");
    }
    directory
}

fn window(id: SessionId, start: u64, end: u64, before: Option<u64>) -> HistoryWindow {
    let mut meta = SessionMeta::new(id, "test".into(), Owner::current());
    let events = (start..=end)
        .map(|cursor| event(id, cursor))
        .collect::<Vec<_>>();
    for event in &events {
        meta.apply_event(event);
    }
    meta.revision = Cursor::from_raw(end);
    HistoryWindow {
        session: meta.into(),
        events,
        context: vec![],
        before,
        cursor: end,
    }
}

fn event(id: SessionId, cursor: u64) -> Event {
    Event::new(
        id,
        Cursor::from_raw(cursor),
        EventKind::TitleChanged {
            title: format!("title-{cursor}"),
        },
    )
}

fn cursors(window: &HistoryWindow) -> Vec<u64> {
    window
        .events
        .iter()
        .map(|event| event.cursor.raw())
        .collect()
}

async fn open(path: &Path) -> Cache {
    Cache::open_at(path, &ConnectionOptions::default(), DEFAULT_BYTE_LIMIT)
        .await
        .expect("open cache")
}

#[tokio::test]
async fn reopening_preserves_pages_overlap_and_newest_frontier() {
    let directory = temporary_cache();
    let path = directory.path().join("history.sqlite");
    let cache = open(&path).await;
    let id = SessionId::new();
    let newest = window(id, 7, 10, Some(7));
    cache.store_window(&newest).await.expect("newest");
    cache
        .append(id, &[event(id, 11), event(id, 12)])
        .await
        .expect("append");
    let mut old = window(id, 3, 6, None);
    old.context = vec![event(id, 1)];
    cache.store_window(&old).await.expect("older page");
    cache
        .store_window(&window(id, 5, 8, Some(5)))
        .await
        .expect("overlap");
    cache
        .store_window(&newest)
        .await
        .expect("idempotent newest write");
    cache
        .store_window(&window(id, 9, 10, Some(9)))
        .await
        .expect("narrower overlap retains wider safe boundary");
    drop(cache);
    let cache = open(&path).await;
    let latest = cache.load_latest(id).await.expect("load").expect("cached");
    assert_eq!(cursors(&latest), (7..=12).collect::<Vec<_>>());
    assert_eq!(latest.cursor, 12);
    assert_eq!(latest.session.meta.title.as_deref(), Some("title-12"));
    let older = cache
        .load_before(id, 7)
        .await
        .expect("load older")
        .expect("cached older");
    assert_eq!(cursors(&older), vec![3, 4, 5, 6]);
    assert_eq!(older.before, None); // Retention exhaustion, not necessarily cursor one.
    assert_eq!(older.context[0].cursor.raw(), 1);
    let prefix = cache
        .load_before(id, 5)
        .await
        .expect("prefix")
        .expect("covered");
    assert_eq!(cursors(&prefix), vec![3, 4]);
    assert_eq!(prefix.context[0].cursor.raw(), 1);
    assert_eq!(prefix.before, None);
    assert!(cache.load_before(id, 3).await.expect("uncovered").is_none());
    assert!(cache.load_before(id, 0).await.expect("zero").is_none());
    cache.remove(id).await.expect("remove");
    assert!(cache.load_latest(id).await.expect("removed").is_none());
    assert!(
        cache
            .load_before(id, 7)
            .await
            .expect("pages removed")
            .is_none()
    );
}

#[tokio::test]
async fn resync_boundaries_reuse_covering_pages_after_restart_without_bridging_holes() {
    let directory = temporary_cache();
    let path = directory.path().join("history.sqlite");
    let cache = open(&path).await;
    let id = SessionId::new();
    cache
        .store_window(&window(id, 1, 600, None))
        .await
        .expect("full history");
    cache
        .store_window(&window(id, 500, 700, Some(500)))
        .await
        .expect("resync");
    drop(cache);

    let cache = open(&path).await;
    let latest = cache
        .load_latest(id)
        .await
        .expect("latest")
        .expect("cached");
    assert_eq!(cursors(&latest), (500..=700).collect::<Vec<_>>());
    let older = cache
        .load_before(id, 500)
        .await
        .expect("older prefix")
        .expect("cached");
    assert_eq!(cursors(&older), (1..=499).collect::<Vec<_>>());
    assert_eq!(older.before, None);
    assert_eq!(older.cursor, 700);

    // Choose the smallest covering upper, not the newest overlapping page.
    cache
        .store_window(&window(id, 400, 650, Some(400)))
        .await
        .expect("overlap");
    assert_eq!(
        cursors(
            &cache
                .load_before(id, 500)
                .await
                .expect("smallest covering page")
                .expect("cached")
        ),
        (1..=499).collect::<Vec<_>>()
    );
    let db = rusqlite::Connection::open(&path).expect("inspect test cache");
    db.execute("DELETE FROM cache_events WHERE cursor = 300", [])
        .expect("create hole");
    assert!(cache.load_before(id, 500).await.is_err());

    let other = SessionId::new();
    cache
        .store_window(&window(other, 1, 100, None))
        .await
        .expect("earlier island");
    cache
        .store_window(&window(other, 300, 400, Some(300)))
        .await
        .expect("later island");
    assert!(
        cache
            .load_before(other, 250)
            .await
            .expect("no covering range")
            .is_none()
    );
}

#[tokio::test]
async fn namespaces_include_auth_references_without_reading_credentials() {
    use bas::service::tls::ClientTls;
    let directory = temporary_cache();
    let path = directory.path().join("history.sqlite");
    let mut alice: ConnectionOptions = "mtls://example.test".parse().expect("target");
    alice.tls = Some(ClientTls {
        ca: "/nonexistent/ca".into(),
        certificate: "/nonexistent/alice".into(),
        private_key: "/nonexistent/alice-key".into(),
        server_name: None,
    });
    let mut bob = alice.clone();
    bob.tls.as_mut().expect("tls").private_key = "/nonexistent/bob-key".into();
    let a = Cache::open_at(&path, &alice, DEFAULT_BYTE_LIMIT)
        .await
        .expect("alice cache");
    let b = Cache::open_at(&path, &bob, DEFAULT_BYTE_LIMIT)
        .await
        .expect("bob cache");
    let id = SessionId::new();
    a.store_window(&window(id, 1, 2, None))
        .await
        .expect("alice");
    assert!(b.load_latest(id).await.expect("bob absent").is_none());
    b.store_window(&window(id, 5, 6, None)).await.expect("bob");
    assert_eq!(
        cursors(&a.load_latest(id).await.expect("alice").expect("cached")),
        vec![1, 2]
    );
    a.remove(id).await.expect("remove alice");
    assert!(b.load_latest(id).await.expect("bob unaffected").is_some());
}

#[tokio::test]
async fn appends_never_bridge_gaps_or_promote_context() {
    let directory = temporary_cache();
    let cache = open(&directory.path().join("history.sqlite")).await;
    let id = SessionId::new();
    let mut initial = window(id, 8, 10, Some(8));
    initial.context = vec![event(id, 1), event(id, 3)];
    cache.store_window(&initial).await.expect("initial");
    cache
        .append(
            id,
            &[
                event(id, 0),
                event(id, 10),
                event(id, 11),
                event(id, 13),
                event(id, 12),
            ],
        )
        .await
        .expect("prefix only");
    let latest = cache.load_latest(id).await.expect("load").expect("cached");
    assert_eq!(cursors(&latest), vec![8, 9, 10, 11]);
    assert_eq!(latest.cursor, 11);
    assert!(
        cache
            .load_before(id, 4)
            .await
            .expect("context is not page")
            .is_none()
    );
    cache
        .append(id, &[event(id, 12), event(id, 13)])
        .await
        .expect("fill gap");
    assert_eq!(
        cache
            .load_latest(id)
            .await
            .expect("load")
            .expect("cached")
            .cursor,
        13
    );
    let mut invalid = window(id, 1, 3, None);
    invalid.events.remove(1);
    assert!(cache.store_window(&invalid).await.is_err());
    assert!(
        cache
            .append(id, &[event(id, 14), event(SessionId::new(), 15)])
            .await
            .is_err()
    );
    assert_eq!(
        cache
            .load_latest(id)
            .await
            .expect("atomic rollback")
            .expect("cached")
            .cursor,
        13
    );
}

#[tokio::test]
async fn empty_history_can_extend_from_zero() {
    let directory = temporary_cache();
    let cache = open(&directory.path().join("history.sqlite")).await;
    let id = SessionId::new();
    let mut empty = window(id, 1, 0, None);
    empty.events.clear();
    cache
        .store_window(&empty)
        .await
        .expect("empty retained window");
    assert!(
        cache
            .load_latest(id)
            .await
            .expect("load")
            .expect("cached")
            .events
            .is_empty()
    );
    cache.append(id, &[event(id, 1)]).await.expect("extend");
    assert_eq!(
        cursors(&cache.load_latest(id).await.expect("load").expect("cached")),
        vec![1]
    );
}

#[tokio::test]
async fn byte_limit_evicts_lru_and_skips_oversized_sessions() {
    let directory = temporary_cache();
    let path = directory.path().join("history.sqlite");
    let cache = open(&path).await;
    let a = SessionId::new();
    let b = SessionId::new();
    let c = SessionId::new();
    cache.store_window(&window(a, 1, 2, None)).await.expect("a");
    let db = rusqlite::Connection::open(&path).expect("inspect test cache");
    let size: i64 = db
        .query_row("SELECT bytes FROM cache_sessions", [], |row| row.get(0))
        .expect("size");
    let cache = Cache::open_at(
        &path,
        &ConnectionOptions::default(),
        (size * 2 + 100) as u64,
    )
    .await
    .expect("small budget");
    cache.store_window(&window(b, 1, 2, None)).await.expect("b");
    cache.load_latest(a).await.expect("touch a");
    cache.store_window(&window(c, 1, 2, None)).await.expect("c");
    assert!(cache.load_latest(b).await.expect("b evicted").is_none());
    assert!(cache.load_latest(a).await.expect("a recent").is_some());
    assert!(cache.load_latest(c).await.expect("c written").is_some());
    let mut oversized = window(b, 1, 2, None);
    oversized.events[0].kind = EventKind::TitleChanged {
        title: "x".repeat(size as usize * 3),
    };
    cache
        .store_window(&oversized)
        .await
        .expect("oversize is nonfatal");
    assert!(
        cache
            .load_latest(b)
            .await
            .expect("oversized skipped")
            .is_none()
    );
    assert!(cache.load_latest(a).await.expect("a survives").is_some());
    let total: i64 = db
        .query_row("SELECT SUM(bytes) FROM cache_sessions", [], |row| {
            row.get(0)
        })
        .expect("total");
    assert!(total <= size * 2 + 100);
}

#[tokio::test]
async fn concurrent_handles_serialize_writes_without_losing_pages() {
    let directory = temporary_cache();
    let path = directory.path().join("history.sqlite");
    let a = open(&path).await;
    let b = open(&path).await;
    let id = SessionId::new();
    let latest = window(id, 7, 9, Some(7));
    let older = window(id, 4, 6, None);
    let (first, second) = tokio::join!(a.store_window(&latest), b.store_window(&older));
    first.expect("first writer");
    second.expect("second writer");
    assert_eq!(
        a.load_latest(id)
            .await
            .expect("latest")
            .expect("cached")
            .cursor,
        9
    );
    assert_eq!(
        b.load_before(id, 7)
            .await
            .expect("older")
            .expect("cached")
            .cursor,
        9
    );
}

#[tokio::test]
async fn older_snapshot_high_water_never_acknowledges_missing_new_events() {
    let directory = temporary_cache();
    let cache = open(&directory.path().join("history.sqlite")).await;
    let id = SessionId::new();
    cache
        .store_window(&window(id, 7, 10, Some(7)))
        .await
        .expect("latest");
    let mut older = window(id, 3, 6, None);
    older.cursor = 30;
    older.session.meta.revision = Cursor::from_raw(30);
    cache
        .store_window(&older)
        .await
        .expect("old page with fresh snapshot");
    let latest = cache
        .load_latest(id)
        .await
        .expect("latest")
        .expect("cached");
    assert_eq!(latest.cursor, 10);
    assert_eq!(cursors(&latest), vec![7, 8, 9, 10]);
    let saved = cache
        .load_before(id, 7)
        .await
        .expect("older")
        .expect("cached");
    assert_eq!(saved.cursor, 30);
    assert_eq!(cursors(&saved), vec![3, 4, 5, 6]);
    cache
        .append(id, &[event(id, 11)])
        .await
        .expect("still accept cursor eleven");
    assert_eq!(
        cache
            .load_latest(id)
            .await
            .expect("latest")
            .expect("cached")
            .cursor,
        11
    );
}

#[tokio::test]
async fn corrupt_payloads_and_other_databases_fail_without_resetting() {
    let directory = temporary_cache();
    let path = directory.path().join("history.sqlite");
    let cache = open(&path).await;
    let id = SessionId::new();
    cache
        .store_window(&window(id, 1, 3, None))
        .await
        .expect("save");
    let db = rusqlite::Connection::open(&path).expect("test db");
    db.execute(
        "UPDATE cache_events SET payload = x'7b' WHERE cursor = 2",
        [],
    )
    .expect("corrupt test event");
    assert!(cache.load_latest(id).await.is_err());
    db.execute_batch("PRAGMA user_version = 99")
        .expect("future version");
    assert!(
        Cache::open_at(&path, &ConnectionOptions::default(), DEFAULT_BYTE_LIMIT)
            .await
            .is_err()
    );
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("unchanged");
    assert_eq!(version, 99);

    let other = directory.path().join("daemon.sqlite");
    let db = rusqlite::Connection::open(&other).expect("other database");
    db.execute_batch(
        "CREATE TABLE daemon_data(value TEXT); INSERT INTO daemon_data VALUES ('keep')",
    )
    .expect("foreign data");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o600))
            .expect("private test database");
    }
    assert!(
        Cache::open_at(&other, &ConnectionOptions::default(), DEFAULT_BYTE_LIMIT)
            .await
            .is_err()
    );
    let value: String = db
        .query_row("SELECT value FROM daemon_data", [], |row| row.get(0))
        .expect("foreign data preserved");
    assert_eq!(value, "keep");
}

#[cfg(unix)]
#[tokio::test]
async fn private_paths_reject_symlinks_and_never_chmod_existing_directories() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = temporary_cache();
    let root = directory.path().join("private");
    let path = root.join("history.sqlite");
    let _cache = open(&path).await;
    assert_eq!(
        std::fs::metadata(&root).expect("dir").permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).expect("db").permissions().mode() & 0o777,
        0o600
    );
    let linked = directory.path().join("linked.sqlite");
    symlink(&path, &linked).expect("symlink");
    assert!(
        Cache::open_at(&linked, &ConnectionOptions::default(), DEFAULT_BYTE_LIMIT)
            .await
            .is_err()
    );
    let linked_dir = directory.path().join("linked-dir");
    symlink(&root, &linked_dir).expect("directory symlink");
    assert!(
        Cache::open_at(
            linked_dir.join("another.sqlite"),
            &ConnectionOptions::default(),
            DEFAULT_BYTE_LIMIT
        )
        .await
        .is_err()
    );
    let public = directory.path().join("public");
    std::fs::create_dir(&public).expect("public test directory");
    std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).expect("set fixture");
    assert!(
        Cache::open_at(
            public.join("history.sqlite"),
            &ConnectionOptions::default(),
            DEFAULT_BYTE_LIMIT
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::metadata(&public)
            .expect("public")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push("-wal");
    symlink(directory.path().join("missing"), &sidecar).expect("sidecar symlink");
    assert!(
        Cache::open_at(&path, &ConnectionOptions::default(), DEFAULT_BYTE_LIMIT)
            .await
            .is_err()
    );
}
