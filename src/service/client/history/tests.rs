use super::*;
use crate::model::{Cursor, EventKind, Owner, SessionMeta};

fn events(id: SessionId, cursors: impl IntoIterator<Item = u64>) -> Vec<pb::Event> {
    cursors
        .into_iter()
        .map(|cursor| {
            (&Event::new(
                id,
                Cursor::from_raw(cursor),
                EventKind::TitleChanged {
                    title: format!("{cursor}"),
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
    let decoded = decode_window(id, None, wire.clone()).expect("safe window");
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
    let decoded = decode_window(id, Some(8), wire).expect("older window");
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
            decode_window(id, None, wire)
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
    task.abort();
    assert_eq!(
        result.expect_err("no history window RPC").code(),
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
}
