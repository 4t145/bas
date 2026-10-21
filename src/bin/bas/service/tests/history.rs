use super::*;
use crate::storage::{Memory, Storage};
use bas::{
    acp,
    model::{Event, EventKind, SessionId, SessionMeta},
    service::Client,
};

const TIMEOUT: Duration = Duration::from_secs(5);

struct Fixture {
    registry: Arc<Registry>,
    client: Client,
    task: tokio::task::JoinHandle<()>,
    session: SessionId,
    events: Vec<Event>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fixture(kinds: Vec<EventKind>) -> Fixture {
    let storage = Arc::new(Memory::new(
        NonZeroUsize::new(kinds.len() + 1).expect("capacity"),
    ));
    let mut meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    meta.status = bas::model::SessionStatus::Stopped;
    storage
        .create(meta.clone(), None)
        .await
        .expect("create stored session");
    let events: Vec<_> = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            let event = Event::new(meta.id, Cursor::from_raw(index as u64 + 1), kind);
            meta.apply_event(&event);
            event
        })
        .collect();
    storage
        .commit(meta.clone(), &events, None)
        .await
        .expect("commit");
    let mut registry = Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage,
    );
    registry.recover().await.expect("recover stopped session");
    let registry = Arc::new(registry);
    let service = Service::new(registry.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service.into_server())
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .expect("server");
    });
    let client = Client::connect_at(address).await.expect("client");
    Fixture {
        registry,
        client,
        task,
        session: meta.id,
        events,
    }
}

fn title(index: usize) -> EventKind {
    EventKind::TitleChanged {
        title: index.to_string(),
    }
}

fn chunk(id: &str, text: &str) -> EventKind {
    let update: acp::SessionUpdate = serde_json::from_value(serde_json::json!({
        "sessionUpdate": "agent_message_chunk",
        "messageId": id,
        "content": { "type": "text", "text": text }
    }))
    .expect("message");
    EventKind::Acp(update)
}

#[tokio::test]
async fn client_windows_expand_dependencies_and_page_back_without_rewriting_events() {
    let mut fixture = fixture(vec![
        title(0),
        chunk("old", "hello"),
        title(2),
        chunk("old", " world"),
    ])
    .await;
    let recent = fixture
        .client
        .history_window(fixture.session, None, 1, TIMEOUT)
        .await
        .expect("window");
    assert_eq!(recent.events, fixture.events[1..]);
    assert_eq!(recent.before, Some(2));
    assert_eq!(recent.cursor, 4);
    assert_eq!(recent.session.meta.revision.raw(), 4);
    let older = fixture
        .client
        .history_window(fixture.session, recent.before, 1, TIMEOUT)
        .await
        .expect("older");
    assert_eq!(older.events, fixture.events[..1]);
    assert_eq!(older.before, None);
    assert_eq!(older.cursor, 4);
    let raw = fixture
        .client
        .load_history(pb::LoadHistoryRequest {
            session: fixture.session.to_string(),
            after: Some(3),
            limit: 1,
        })
        .await
        .expect("raw history unchanged")
        .into_inner();
    assert_eq!(raw.events.len(), 1);
    assert_eq!(raw.events[0].cursor, 4);
    fixture.registry.shutdown().await;
}

#[tokio::test]
async fn client_windows_preserve_payloads_and_snapshot_across_pages() {
    let mut fixture = fixture((0..600).map(title).collect()).await;
    let recent = fixture
        .client
        .history_window(fixture.session, None, 256, TIMEOUT)
        .await
        .expect("recent");
    assert_eq!(recent.events, fixture.events[344..]);
    assert_eq!(recent.before, Some(345));
    assert_eq!(recent.cursor, 600);
    assert_eq!(recent.session.meta.id, fixture.session);
    assert_eq!(recent.session.meta.revision.raw(), 600);
    let older = fixture
        .client
        .history_window(fixture.session, recent.before, 256, TIMEOUT)
        .await
        .expect("older");
    assert_eq!(older.events, fixture.events[88..344]);
    assert_eq!(older.before, Some(89));
    assert_eq!(older.cursor, recent.cursor);
    assert_eq!(older.session, recent.session);
    let oldest = fixture
        .client
        .history_window(fixture.session, older.before, 256, TIMEOUT)
        .await
        .expect("oldest");
    assert_eq!(oldest.events, fixture.events[..88]);
    assert_eq!(oldest.before, None);
    assert_eq!(oldest.cursor, recent.cursor);
    assert_eq!(oldest.session, recent.session);
    fixture.registry.shutdown().await;
}

#[tokio::test]
async fn oversized_safe_window_errors_instead_of_returning_half_a_message() {
    let text = "x".repeat(bas::service::MAX_MESSAGE_SIZE / 2);
    let mut fixture = fixture(vec![chunk("large", &text), chunk("large", &text)]).await;
    let error = fixture
        .client
        .history_window(fixture.session, None, 1, TIMEOUT)
        .await
        .expect_err("complete message does not fit");
    assert_eq!(error.code(), Code::ResourceExhausted);
    fixture.registry.shutdown().await;
}

#[tokio::test]
async fn window_cursor_resumes_events_committed_before_attach() {
    let registry = Arc::new(Registry::with_storage(
        Arc::new(Catalog::with_defaults()),
        Arc::new(LocalHost::default()),
        Limits::default(),
        Arc::new(Memory::new(NonZeroUsize::new(32).expect("capacity"))),
    ));
    let session = create(&registry).await;
    let service = Service::new(registry.clone());
    let window = service
        .load_history_window(Request::new(pb::LoadHistoryWindowRequest {
            session: session.id().to_string(),
            before: None,
            limit: 1,
        }))
        .await
        .expect("window")
        .into_inner();
    assert_eq!(
        window.session.as_ref().expect("snapshot").revision,
        window.cursor
    );
    session
        .set_title("after snapshot")
        .await
        .expect("new event");
    let mut stream = service
        .attach(Request::new(pb::AttachRequest {
            session: session.id().to_string(),
            cursor: Some(window.cursor),
            replay: 0,
        }))
        .await
        .expect("attach")
        .into_inner();
    let event = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .expect("no history gap")
        .expect("event")
        .expect("success");
    assert_eq!(event.cursor, window.cursor + 1);
    registry.shutdown().await;
}
