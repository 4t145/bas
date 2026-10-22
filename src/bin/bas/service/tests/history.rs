use super::*;
use crate::storage::{Memory, Storage};
use bas::{
    acp,
    model::{Event, EventKind, SessionId, SessionMeta},
    service::Client,
};

const TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn history_responses_use_gzip_and_preserve_complete_payloads() {
    let mut fixture = fixture(vec![prompt(0), chunk("answer", &"repeat ".repeat(4096))]).await;
    let response = fixture
        .client
        .load_history_window(pb::LoadHistoryWindowRequest {
            session: fixture.session.to_string(),
            before: None,
            limit: 1,
        })
        .await
        .expect("compressed history");
    assert_eq!(
        response
            .metadata()
            .get("grpc-encoding")
            .map(|value| value.to_str().expect("encoding")),
        Some("gzip"),
    );
    let events = response
        .into_inner()
        .events
        .into_iter()
        .map(TryInto::try_into)
        .collect::<std::result::Result<Vec<Event>, _>>()
        .expect("decoded full payloads");
    assert_eq!(events, fixture.events);
}

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

fn prompt(index: usize) -> EventKind {
    EventKind::Acp(
        serde_json::from_value(serde_json::json!({
            "sessionUpdate":"user_message_chunk", "messageId":format!("prompt-{index}"),
            "content":{"type":"text","text":"question"}
        }))
        .expect("user prompt"),
    )
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
        prompt(0),
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
    assert_eq!(recent.cursor, 5);
    assert_eq!(recent.session.meta.revision.raw(), 5);
    let older = fixture
        .client
        .history_window(fixture.session, recent.before, 1, TIMEOUT)
        .await
        .expect("older");
    assert_eq!(older.events, fixture.events[..1]);
    assert_eq!(older.before, None);
    assert_eq!(older.cursor, 5);
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
    let mut fixture = fixture(
        (0..600)
            .map(|index| {
                if index % 200 == 0 {
                    prompt(index)
                } else {
                    title(index)
                }
            })
            .collect(),
    )
    .await;
    let recent = fixture
        .client
        .history_window(fixture.session, None, 1, TIMEOUT)
        .await
        .expect("recent");
    assert_eq!(recent.events, fixture.events[400..]);
    assert_eq!(recent.before, Some(401));
    assert_eq!(recent.cursor, 600);
    assert_eq!(recent.session.meta.id, fixture.session);
    assert_eq!(recent.session.meta.revision.raw(), 600);
    let overview = fixture
        .client
        .history_overview(fixture.session, TIMEOUT)
        .await
        .expect("complete prompt markers");
    assert_eq!(overview.cursor, 600);
    assert_eq!(
        overview
            .entries
            .iter()
            .map(|entry| entry.cursor)
            .collect::<Vec<_>>(),
        [1, 201, 401]
    );
    let direct = fixture
        .client
        .history_window_at(fixture.session, 1, TIMEOUT)
        .await
        .expect("jump directly from latest to oldest");
    assert_eq!(direct.window.events, fixture.events[..200]);
    assert_eq!(direct.window.cursor, 600);
    assert_eq!(direct.after, Some(201));
    let forward = fixture
        .client
        .history_window_at(
            fixture.session,
            direct.after.expect("forward cursor"),
            TIMEOUT,
        )
        .await
        .expect("next page");
    assert_eq!(forward.window.events, fixture.events[200..400]);
    assert_eq!(forward.after, Some(401));
    let containing = fixture
        .client
        .history_window_at(fixture.session, 250, TIMEOUT)
        .await
        .expect("containing middle turn");
    assert_eq!(containing, forward);
    assert_eq!(
        fixture
            .client
            .history_window_at(fixture.session, 601, TIMEOUT)
            .await
            .expect_err("future cursor")
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        fixture
            .client
            .history_overview(SessionId::new(), TIMEOUT)
            .await
            .expect_err("missing session")
            .code(),
        Code::NotFound
    );
    let older = fixture
        .client
        .history_window(fixture.session, recent.before, 1, TIMEOUT)
        .await
        .expect("older");
    assert_eq!(older.events, fixture.events[200..400]);
    assert_eq!(older.before, Some(201));
    assert_eq!(older.cursor, recent.cursor);
    assert_eq!(older.session, recent.session);
    let oldest = fixture
        .client
        .history_window(fixture.session, older.before, 1, TIMEOUT)
        .await
        .expect("oldest");
    assert_eq!(oldest.events, fixture.events[..200]);
    assert_eq!(oldest.before, None);
    assert_eq!(oldest.cursor, recent.cursor);
    assert_eq!(oldest.session, recent.session);
    fixture.registry.shutdown().await;
}

#[tokio::test]
async fn oversized_safe_window_errors_instead_of_returning_half_a_message() {
    let text = "x".repeat(bas::service::MAX_MESSAGE_SIZE / 2);
    let mut fixture = fixture(vec![
        EventKind::TurnStarted {
            turn: bas::model::TurnId::new(),
        },
        prompt(0),
        chunk("large-a", &text),
        chunk("large-b", &text),
    ])
    .await;
    let error = fixture
        .client
        .history_window(fixture.session, None, 1, TIMEOUT)
        .await
        .expect_err("complete message does not fit");
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert_eq!(
        fixture
            .client
            .history_window_at(fixture.session, 1, TIMEOUT)
            .await
            .expect_err("oversized location")
            .code(),
        Code::ResourceExhausted
    );
    assert_eq!(
        fixture
            .client
            .history_overview(fixture.session, TIMEOUT)
            .await
            .expect("metadata does not contain bodies")
            .entries
            .len(),
        1
    );
    fixture.registry.shutdown().await;
}

struct NoReadStartup;

#[async_trait::async_trait]
impl crate::agent::Driver for NoReadStartup {
    fn kind(&self) -> crate::agent::Kind {
        crate::agent::Kind::Acp
    }

    async fn launch(
        &self,
        _: crate::agent::LaunchRequest,
    ) -> Result<Box<dyn crate::agent::Agent>, crate::agent::Error> {
        panic!("history reads must not launch ACP")
    }

    async fn restore(
        &self,
        _: crate::agent::LaunchRequest,
        _: String,
    ) -> Result<Box<dyn crate::agent::Agent>, crate::agent::Error> {
        panic!("history reads must not restore ACP")
    }
}

#[tokio::test]
async fn navigation_reads_never_launch_or_restore_acp() {
    let storage = Arc::new(Memory::new(NonZeroUsize::new(10).expect("capacity")));
    let mut meta = SessionMeta::new(SessionId::new(), "read-only".into(), Owner::current());
    meta.status = bas::model::SessionStatus::Idle;
    storage
        .create(
            meta.clone(),
            Some(crate::storage::ResumeData::acp("native-session")),
        )
        .await
        .expect("stored ACP session");
    let event = Event::new(meta.id, Cursor::from_raw(1), prompt(0));
    meta.apply_event(&event);
    storage
        .commit(meta.clone(), &[event], None)
        .await
        .expect("stored prompt");
    let mut catalog = Catalog::new();
    catalog
        .insert(
            crate::agent::AgentSpec::acp(
                "read-only",
                crate::agent::Command::new("unused", std::iter::empty::<String>()),
            ),
            Arc::new(NoReadStartup),
        )
        .expect("ACP driver");
    let mut registry = Registry::with_storage(
        Arc::new(catalog),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage,
    );
    registry.recover().await.expect("recover metadata");
    let registry = Arc::new(registry);
    let session = registry.get(meta.id).await.expect("session");
    let before = session.snapshot();
    let service = Service::new(registry.clone());
    let overview = service
        .load_history_overview(Request::new(pb::LoadHistoryOverviewRequest {
            session: meta.id.to_string(),
        }))
        .await
        .expect("overview")
        .into_inner();
    assert_eq!(overview.entries.len(), 1);
    let location = service
        .load_history_window_at(Request::new(pb::LoadHistoryWindowAtRequest {
            session: meta.id.to_string(),
            cursor: 1,
        }))
        .await
        .expect("location")
        .into_inner();
    assert_eq!(location.window.expect("window").events.len(), 1);
    assert_eq!(session.snapshot(), before);
    assert_eq!(
        service
            .load_history_window_at(Request::new(pb::LoadHistoryWindowAtRequest {
                session: meta.id.to_string(),
                cursor: 0
            }))
            .await
            .expect_err("zero cursor")
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        service
            .load_history_overview(Request::new(pb::LoadHistoryOverviewRequest {
                session: "invalid".into()
            }))
            .await
            .expect_err("invalid session")
            .code(),
        Code::InvalidArgument
    );
    registry.shutdown().await;
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
