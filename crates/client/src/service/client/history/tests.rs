use super::*;
use crate::model::{Cursor, EventKind, Owner, SessionMeta};

fn events(id: SessionId, cursors: impl IntoIterator<Item = u64>) -> Vec<pb::Event> {
    cursors
        .into_iter()
        .map(|cursor| {
            (&Event::new(
                id,
                Cursor::from_raw(cursor),
                if [5, 8].contains(&cursor) {
                    EventKind::Acp(
                        serde_json::from_value(serde_json::json!({
                            "sessionUpdate":"user_message_chunk", "messageId":format!("p-{cursor}"),
                            "content":{"type":"text","text":"prompt"}
                        }))
                        .expect("prompt"),
                    )
                } else {
                    EventKind::TitleChanged {
                        title: format!("{cursor}"),
                    }
                },
            ))
                .into()
        })
        .collect()
}

fn window(id: SessionId) -> pb::LoadHistoryWindowResponse {
    let mut meta = SessionMeta::new(id, "test".into(), Owner::current());
    meta.revision = Cursor::from_raw(10);
    pb::LoadHistoryWindowResponse {
        session: Some((&SessionSnapshot::from(meta)).try_into().expect("metadata")),
        events: events(id, 8..=10),
        context: events(id, [2, 5]),
        before: Some(8),
        cursor: 10,
    }
}

#[test]
fn window_preserves_payloads_context_and_snapshot() {
    let id = SessionId::new();
    let wire = window(id);
    let decoded = HistoryWindow::decode(id, None, wire.clone()).expect("safe window");
    assert_eq!(
        decoded
            .events
            .iter()
            .map(pb::Event::from)
            .collect::<Vec<_>>(),
        wire.events
    );
    assert_eq!(
        decoded
            .context
            .iter()
            .map(pb::Event::from)
            .collect::<Vec<_>>(),
        wire.context
    );
    assert_eq!(
        pb::Session::try_from(&decoded.session).expect("session payload"),
        wire.session.expect("session")
    );
    assert_eq!(decoded.cursor, decoded.session.meta.revision.raw());
    assert_eq!(decoded.before, Some(8));
    let serialized = serde_json::to_vec(&decoded).expect("cache serialization");
    assert_eq!(
        serde_json::from_slice::<HistoryWindow>(&serialized).expect("cache decoding"),
        decoded
    );
}

#[test]
fn older_window_ends_before_request_not_at_latest_revision() {
    let id = SessionId::new();
    let mut wire = window(id);
    wire.events = events(id, 5..=7);
    wire.context = events(id, [2]);
    wire.before = Some(5);
    let decoded = HistoryWindow::decode(id, Some(8), wire).expect("older window");
    assert_eq!(decoded.cursor, 10);
    assert_eq!(decoded.events.last().expect("event").cursor.raw(), 7);
}

#[test]
fn malformed_windows_are_rejected() {
    let id = SessionId::new();
    let original = window(id);
    let mut cases = Vec::new();
    let mut wire = original.clone();
    wire.cursor = 11;
    cases.push(wire);
    let mut wire = original.clone();
    wire.events = events(id, [8, 10]);
    cases.push(wire);
    let mut wire = original.clone();
    wire.context = events(id, [8]);
    cases.push(wire);
    let mut wire = original.clone();
    wire.before = Some(9);
    cases.push(wire);
    let mut wire = original.clone();
    wire.events = events(SessionId::new(), 8..=10);
    cases.push(wire);
    let mut wire = original.clone();
    wire.events.clear();
    cases.push(wire);
    let mut wire = original.clone();
    wire.session = None;
    cases.push(wire);
    let mut wire = original.clone();
    wire.session = window(SessionId::new()).session;
    cases.push(wire);
    let mut wire = original.clone();
    wire.events = events(id, [0, 9, 10]);
    cases.push(wire);
    let mut wire = original.clone();
    wire.events = events(id, [9, 8, 10]);
    cases.push(wire);
    let mut wire = original.clone();
    wire.events = events(id, [8, 8, 10]);
    cases.push(wire);
    let mut wire = original;
    wire.events = events(id, 8..=9);
    cases.push(wire);
    for wire in cases {
        assert_eq!(
            HistoryWindow::decode(id, None, wire)
                .expect_err("invalid response")
                .code(),
            tonic::Code::DataLoss
        );
    }
}

#[tokio::test]
async fn window_rpc_errors_are_propagated() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_routes(tonic::service::Routes::default())
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .expect("server");
    });
    let mut client = Client::connect_at(address).await.expect("client");
    let result = client
        .history_window(SessionId::new(), None, 1, Duration::from_secs(5))
        .await;
    let overview = client
        .history_overview(SessionId::new(), Duration::from_secs(5))
        .await;
    let location = client
        .history_window_at(SessionId::new(), 1, Duration::from_secs(5))
        .await;
    task.abort();
    assert_eq!(
        result.expect_err("no history window RPC").code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        overview.expect_err("no overview RPC").code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        location.expect_err("no location RPC").code(),
        tonic::Code::Unimplemented
    );
}

#[tokio::test]
async fn each_history_window_rpc_bounds_a_stalled_request() {
    // A listening socket that never speaks HTTP/2 stalls every request without
    // requiring a second fake implementation of the complete Bas service.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let endpoint = tonic::transport::Endpoint::from_shared(format!(
        "http://{}",
        listener.local_addr().expect("address")
    ))
    .expect("endpoint");
    let mut client = Client::new(endpoint.connect_lazy());
    let session = SessionId::new();
    let timeout = Duration::from_millis(25);
    for before in [None, Some(8)] {
        let status = client
            .history_window(session, before, 1, timeout)
            .await
            .expect_err("stalled RPC must fail");
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }
    assert_eq!(
        client
            .history_overview(session, timeout)
            .await
            .expect_err("stalled overview")
            .code(),
        tonic::Code::DeadlineExceeded
    );
    assert_eq!(
        client
            .history_window_at(session, 1, timeout)
            .await
            .expect_err("stalled location")
            .code(),
        tonic::Code::DeadlineExceeded
    );
    assert_eq!(
        client
            .history_window_at(session, 0, timeout)
            .await
            .expect_err("zero cursor rejected locally")
            .code(),
        tonic::Code::InvalidArgument
    );
}

#[test]
fn overview_validates_identity_order_bounds_and_preview_size() {
    let id = SessionId::new();
    let original = pb::LoadHistoryOverviewResponse {
        session: id.to_string(),
        cursor: 10,
        entries: [2, 5, 8]
            .into_iter()
            .map(|cursor| pb::HistoryOverviewEntry {
                cursor,
                timestamp_millis: 1234,
                preview: format!("prompt-{cursor}"),
            })
            .collect(),
    };
    let overview = HistoryOverview::decode(id, original.clone()).expect("complete catalog");
    assert_eq!(overview.entries.len(), 3);
    assert_eq!(overview.entries[0].cursor, 2);
    assert_eq!(overview.entries[0].timestamp_millis, 1234);
    assert_eq!(overview.entries[0].preview, "prompt-2");
    assert_eq!(
        serde_json::from_slice::<HistoryOverview>(&serde_json::to_vec(&overview).expect("encode"))
            .expect("decode"),
        overview
    );
    let mut cases = Vec::new();
    let mut wire = original.clone();
    wire.session = SessionId::new().to_string();
    cases.push(wire);
    let mut wire = original.clone();
    wire.entries[0].cursor = 0;
    cases.push(wire);
    let mut wire = original.clone();
    wire.entries[1].cursor = 2;
    cases.push(wire);
    let mut wire = original.clone();
    wire.entries.swap(0, 1);
    cases.push(wire);
    let mut wire = original.clone();
    wire.cursor = 7;
    cases.push(wire);
    let mut wire = original.clone();
    wire.entries[0].timestamp_millis = -1;
    cases.push(wire);
    let mut wire = original;
    wire.entries[0].preview = "界".repeat(crate::model::history::PROMPT_PREVIEW_CHARS + 1);
    cases.push(wire);
    for wire in cases {
        assert_eq!(
            HistoryOverview::decode(id, wire)
                .expect_err("invalid overview")
                .code(),
            tonic::Code::DataLoss
        );
    }
}

#[test]
fn directly_addressed_windows_validate_containment_and_forward_boundary() {
    let id = SessionId::new();
    let mut raw = window(id);
    raw.events = events(id, 5..=7);
    raw.context = events(id, [2]);
    raw.before = Some(5);
    let wire = pb::LoadHistoryWindowAtResponse {
        window: Some(raw),
        after: Some(8),
    };
    let location = HistoryLocation::decode(id, 6, wire.clone()).expect("containing turn");
    assert_eq!(location.window.events.len(), 3);
    assert_eq!(location.window.cursor, 10);
    assert_eq!(location.after, Some(8));
    assert_eq!(
        serde_json::from_slice::<HistoryLocation>(&serde_json::to_vec(&location).expect("encode"))
            .expect("decode"),
        location
    );
    for cursor in [0, 4, 8, 11] {
        assert_eq!(
            HistoryLocation::decode(id, cursor, wire.clone())
                .expect_err("not contained")
                .code(),
            tonic::Code::DataLoss
        );
    }
    for after in [None, Some(0), Some(6), Some(9), Some(11)] {
        let mut invalid = wire.clone();
        invalid.after = after;
        assert_eq!(
            HistoryLocation::decode(id, 6, invalid)
                .expect_err("bad next boundary")
                .code(),
            tonic::Code::DataLoss
        );
    }
    let mut invalid = wire.clone();
    invalid.window.as_mut().expect("window").before = None;
    assert!(HistoryLocation::decode(id, 6, invalid).is_err());
    assert!(
        HistoryLocation::decode(
            id,
            6,
            pb::LoadHistoryWindowAtResponse {
                window: None,
                after: None
            }
        )
        .is_err()
    );
    assert!(
        HistoryLocation::decode(
            id,
            9,
            pb::LoadHistoryWindowAtResponse {
                window: Some(window(id)),
                after: None
            }
        )
        .is_ok()
    );
}
