use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Scenario {
    Unimplemented,
    Unavailable,
    InvalidPayload,
    WrongSession,
}

pub(super) async fn load(
    service: &Service,
    scenario: Scenario,
    _: pb::LoadHistoryWindowRequest,
) -> Result<Response<pb::LoadHistoryWindowResponse>, Status> {
    match scenario {
        Scenario::Unimplemented => return Err(Status::unimplemented("history window RPC missing")),
        Scenario::Unavailable => return Err(Status::unavailable("history interrupted")),
        _ => {}
    }
    let mut events: Vec<pb::Event> = service.history[44..].iter().map(Into::into).collect();
    match scenario {
        Scenario::InvalidPayload => events[0] = pb::Event::default(),
        Scenario::WrongSession => events[0].session = SessionId::new().to_string(),
        _ => {}
    }
    Ok(Response::new(pb::LoadHistoryWindowResponse {
        session: Some(
            pb::Session::try_from(&SessionSnapshot {
                meta: service.session.clone(),
                runtime: service.runtime.clone(),
            })
            .expect("metadata"),
        ),
        events,
        context: Vec::new(),
        before: Some(45),
        cursor: 300,
    }))
}

async fn assert_terminal_window_error(scenario: Scenario) {
    let fixture = fixture(super::Scenario::History(scenario)).await;
    // Channel closure proves the watcher terminates instead of retrying.
    let updates = tokio::time::timeout(Duration::from_secs(3), collect(&fixture))
        .await
        .expect("failed window terminates immediately");
    assert!(matches!(
        &updates[..],
        [
            Update::Stream {
                epoch: 1,
                state: StreamState::Loading
            },
            Update::Stream {
                epoch: 1,
                state: StreamState::Unavailable(_)
            },
        ]
    ));
    assert_eq!(
        *fixture.history_requests.lock().expect("requests"),
        vec![None]
    );
    assert!(
        fixture
            .raw_history_requests
            .lock()
            .expect("raw requests")
            .is_empty()
    );
    assert!(fixture.attachments.lock().expect("attachments").is_empty());
}

#[tokio::test]
async fn missing_window_rpc_is_immediately_unavailable_without_raw_history_or_attach() {
    assert_terminal_window_error(Scenario::Unimplemented).await;
}

#[tokio::test]
async fn unavailable_window_stops_without_retrying_or_attaching() {
    assert_terminal_window_error(Scenario::Unavailable).await;
}

#[tokio::test]
async fn malformed_window_is_not_published_or_attached() {
    for scenario in [Scenario::InvalidPayload, Scenario::WrongSession] {
        assert_terminal_window_error(scenario).await;
    }
}
