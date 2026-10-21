use super::*;

pub(super) fn late_event(session: SessionId) -> Event {
    Event::new(
        session,
        Cursor::from_raw(301),
        EventKind::Acp(acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                "old-tool",
                acp::ToolCallUpdateFields::new().title("finished"),
            ),
        )),
    )
}

pub(super) fn late_window(
    service: &Service,
    request: pb::LoadHistoryWindowRequest,
) -> Result<Response<pb::LoadHistoryWindowResponse>, Status> {
    let mut calls = service.history_requests.lock().expect("requests");
    calls.push(request.before);
    let refreshed = calls.len() > 1;
    let mut events = service.history.clone();
    events[0] = Event::new(
        service.session.id,
        Cursor::from_raw(1),
        EventKind::Acp(acp::SessionUpdate::ToolCall(acp::ToolCall::new(
            "old-tool", "original",
        ))),
    );
    if refreshed {
        events.push(late_event(service.session.id));
    } else {
        events.drain(..298);
    }
    let mut meta = service.session.clone();
    meta.revision = Cursor::from_raw(if refreshed { 301 } else { 300 });
    Ok(Response::new(pb::LoadHistoryWindowResponse {
        cursor: meta.revision.raw(),
        session: Some(
            pb::Session::try_from(&SessionSnapshot {
                meta,
                runtime: service.runtime.clone(),
            })
            .expect("metadata"),
        ),
        events: events.iter().map(Into::into).collect(),
        context: Vec::new(),
        before: (!refreshed).then_some(299),
    }))
}

#[tokio::test]
async fn late_tool_update_refreshes_dependencies_before_exposing_the_update() {
    let fixture = fixture(Scenario::LateTool).await;
    let updates = collect(&fixture).await;
    assert!(
        !updates
            .iter()
            .any(|update| matches!(update, Update::Event { .. }))
    );
    let windows = updates
        .iter()
        .filter_map(|update| match update {
            Update::HistoryWindow { window, .. } => Some(window),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].events.len(), 2);
    assert_eq!(windows[1].events.len(), 301);
    assert_eq!(windows[1].events[0].cursor.raw(), 1);
    assert_eq!(windows[1].events[300].cursor.raw(), 301);
    assert_eq!(
        *fixture.attachments.lock().expect("attachments"),
        vec![Some(300), Some(301)]
    );
}

async fn cache(path: &std::path::Path) -> crate::connection::cache::Cache {
    crate::connection::cache::Cache::open_at(path, &Target::default(), 8 * 1024 * 1024)
        .await
        .expect("temporary cache")
}

async fn window(fixture: &Fixture) -> HistoryWindow {
    fixture
        .client
        .clone()
        .history_window(fixture.session, None, HISTORY_TURNS, REQUEST_TIMEOUT)
        .await
        .expect("window")
}

#[tokio::test]
async fn cold_history_reads_one_recent_window_and_attaches_at_its_frontier() {
    let fixture = fixture(Scenario::Window).await;
    let updates = collect(&fixture).await;
    let windows = updates
        .iter()
        .filter_map(|update| match update {
            Update::HistoryWindow {
                window,
                cached: false,
                ..
            } => Some(window),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].events.len(), 2);
    assert_eq!(windows[0].before, Some(299));
    assert!(
        fixture
            .raw_history_requests
            .lock()
            .expect("raw requests")
            .is_empty()
    );
    assert_eq!(
        *fixture.history_requests.lock().expect("requests"),
        vec![None]
    );
    assert_eq!(
        *fixture.attachments.lock().expect("attachments"),
        vec![Some(300), Some(301)]
    );
}

#[tokio::test]
async fn unavailable_window_does_not_fall_back_to_full_history() {
    let fixture = fixture(Scenario::WindowUnavailable).await;
    let updates = collect(&fixture).await;
    assert!(matches!(
        updates.last(),
        Some(Update::Stream {
            state: StreamState::Unavailable(_),
            ..
        })
    ));
    assert!(
        fixture
            .raw_history_requests
            .lock()
            .expect("requests")
            .is_empty()
    );
    assert_eq!(
        *fixture.history_requests.lock().expect("requests"),
        vec![None]
    );
    assert!(fixture.attachments.lock().expect("attachments").is_empty());
}

#[tokio::test]
async fn reopening_disk_history_attaches_without_loading_history_again() {
    let fixture = fixture(Scenario::Window).await;
    let directory = tempfile::tempdir().expect("cache directory");
    let path = directory.path().join("private/history.sqlite");
    let disk = cache(&path).await;
    disk.store_window(&window(&fixture).await)
        .await
        .expect("store");
    drop(disk);
    fixture.history_requests.lock().expect("requests").clear();
    let (store, writer) = crate::connection::history::Store::new(Some(cache(&path).await));
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(crate::connection::permissions::watch_cached(
        fixture.client.clone(),
        updates,
        fixture.session,
        3,
        None,
        store,
    ));
    tokio::time::timeout(Duration::from_secs(3), async {
        assert!(matches!(
            events.recv().await,
            Some(Update::HistoryWindow {
                cached: true,
                epoch: 3,
                ..
            })
        ));
        loop {
            match events.recv().await.expect("watch update") {
                Update::Stream {
                    state: StreamState::Live,
                    ..
                } => break,
                Update::HistoryWindow { .. } => {
                    panic!("duplicate history")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("reopens");
    assert_eq!(
        fixture.attachments.lock().expect("attachments")[0],
        Some(300)
    );
    assert!(
        fixture
            .history_requests
            .lock()
            .expect("requests")
            .is_empty()
    );
    task.abort();
    let _ = task.await;
    writer.await.expect("writer drains");
}

#[tokio::test]
async fn cached_history_is_visible_before_the_permission_connection_finishes() {
    let fixture = fixture(Scenario::SlowPermissions).await;
    let directory = tempfile::tempdir().expect("cache directory");
    let cache = cache(&directory.path().join("private/history.sqlite")).await;
    cache
        .store_window(&window(&fixture).await)
        .await
        .expect("store");
    let (store, writer) = crate::connection::history::Store::new(Some(cache));
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(crate::connection::permissions::watch_cached(
        fixture.client.clone(),
        updates,
        fixture.session,
        8,
        None,
        store,
    ));
    let first = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("cache does not wait for permissions")
        .expect("update");
    assert!(matches!(
        first,
        Update::HistoryWindow {
            epoch: 8,
            cached: true,
            ..
        }
    ));
    assert!(fixture.attachments.lock().expect("attachments").is_empty());
    task.abort();
    let _ = task.await;
    writer.await.expect("writer drains");
}

#[tokio::test]
async fn older_page_is_cache_first_and_keeps_the_requested_epoch() {
    let fixture = fixture(Scenario::Window).await;
    let directory = tempfile::tempdir().expect("cache directory");
    let cache = cache(&directory.path().join("private/history.sqlite")).await;
    let latest = window(&fixture).await;
    cache.store_window(&latest).await.expect("latest");
    let older = fixture
        .client
        .clone()
        .history_window(fixture.session, Some(299), HISTORY_TURNS, REQUEST_TIMEOUT)
        .await
        .expect("older");
    cache.store_window(&older).await.expect("older cache");
    fixture.history_requests.lock().expect("requests").clear();
    let (commands, receiver) = mpsc::unbounded_channel();
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(run_cached(
        fixture.client.clone(),
        receiver,
        updates,
        Some(cache),
    ));
    commands
        .send(Command::OlderHistory {
            session: fixture.session,
            epoch: 17,
            before: 299,
        })
        .expect("command");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Update::OlderHistory {
                epoch,
                before,
                result,
            } = events.recv().await.expect("updates")
            {
                assert_eq!(epoch, 17);
                assert_eq!(before, 299);
                let window = result.expect("cached older");
                assert_eq!(window.events.first().expect("first").cursor.raw(), 297);
                assert_eq!(window.events.last().expect("last").cursor.raw(), 298);
                break;
            }
        }
    })
    .await
    .expect("older response");
    assert!(
        fixture
            .history_requests
            .lock()
            .expect("requests")
            .is_empty()
    );
    drop(commands);
    task.await.expect("shutdown");
}

#[tokio::test]
async fn hot_resume_uses_cursor_without_a_duplicate_snapshot() {
    for cursor in [280, 300] {
        let fixture = fixture(Scenario::Window).await;
        let (store, writer) = crate::connection::history::Store::new(None);
        let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
        let task = tokio::spawn(watch_cached(
            fixture.client.clone(),
            updates,
            fixture.session,
            cursor,
            Some(cursor),
            store,
        ));
        let mut metadata = false;
        while let Some(update) = events.recv().await {
            assert!(!matches!(update, Update::HistoryWindow { .. }));
            if let Update::HistoryMeta { epoch, session } = update {
                assert_eq!(epoch, cursor);
                assert_eq!(session.meta.revision.raw(), 300);
                metadata = true;
            }
        }
        task.await.expect("watch");
        writer.await.expect("writer");
        assert!(metadata);
        assert_eq!(
            fixture.attachments.lock().expect("attachments")[0],
            Some(cursor)
        );
        assert!(
            fixture
                .history_requests
                .lock()
                .expect("requests")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn shutdown_flushes_events_and_forget_cannot_be_undone_by_pending_writes() {
    let fixture = fixture(Scenario::Window).await;
    let directory = tempfile::tempdir().expect("cache directory");
    let path = directory.path().join("private/history.sqlite");
    let cache = cache(&path).await;
    let initial = window(&fixture).await;
    let (store, writer) = crate::connection::history::Store::new(Some(cache.clone()));
    store.window(&initial).await;
    let event = Event::new(
        fixture.session,
        Cursor::from_raw(301),
        EventKind::TitleChanged {
            title: "new".into(),
        },
    );
    store.event(&event).await;
    store.flush().await;
    drop(store);
    writer.await.expect("flush");
    assert_eq!(
        cache
            .load_latest(fixture.session)
            .await
            .expect("read")
            .expect("window")
            .cursor,
        301
    );
    let (store, writer) = crate::connection::history::Store::new(Some(cache.clone()));
    store.forget(fixture.session).await;
    store.window(&initial).await;
    store.event(&event).await;
    drop(store);
    writer.await.expect("forget");
    assert!(
        cache
            .load_latest(fixture.session)
            .await
            .expect("read")
            .is_none()
    );
}
