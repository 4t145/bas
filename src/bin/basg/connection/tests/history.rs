use super::*;
use crate::connection::history::snapshot;

#[derive(Clone, Copy, Debug)]
pub(super) enum Scenario {
    SlowPages,
    HungPage,
    HungMetadata,
    RepeatedCursor,
    BackwardCursor,
    InvalidPayload,
    WrongSession,
    Unavailable,
}

pub(super) async fn load(
    service: &Service,
    scenario: Scenario,
    request: pb::LoadHistoryRequest,
) -> Result<Response<pb::LoadHistoryResponse>, Status> {
    assert_eq!(request.session, service.session.id.to_string());
    assert_eq!(request.limit, HISTORY_PAGE_SIZE);
    if matches!(scenario, Scenario::SlowPages) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    if request.after.is_some() {
        match scenario {
            Scenario::HungPage => return std::future::pending().await,
            Scenario::Unavailable => return Err(Status::unavailable("history interrupted")),
            _ => {}
        }
    }
    // Small pages allow six successful RPCs to exceed one deadline, with
    // ample scheduling margin per RPC on the real TCP transport.
    let limit = if matches!(scenario, Scenario::SlowPages) {
        50
    } else {
        request.limit as usize
    };
    let remaining: Vec<_> = service
        .history
        .iter()
        .filter(|event| event.cursor.raw() > request.after.unwrap_or(0))
        .collect();
    let mut events: Vec<pb::Event> = remaining
        .iter()
        .take(limit)
        .map(|event| (*event).into())
        .collect();
    let mut next = (remaining.len() > events.len()).then(|| events.last().expect("page").cursor);
    if let Some(after) = request.after {
        match scenario {
            Scenario::RepeatedCursor => next = Some(after),
            Scenario::BackwardCursor => next = Some(after - 1),
            Scenario::InvalidPayload => events[0] = pb::Event::default(),
            Scenario::WrongSession => {
                events[0] = (&Event::new(
                    SessionId::new(),
                    Cursor::from_raw(after + 1),
                    EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
                        acp::ContentChunk::new("wrong session".into()),
                    )),
                ))
                    .into();
            }
            _ => {}
        }
    }
    Ok(Response::new(pb::LoadHistoryResponse { events, next }))
}

const RPC_TIMEOUT: Duration = Duration::from_millis(500);
const TEST_TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::test]
async fn cumulative_page_time_can_exceed_one_rpc_timeout_and_reports_progress() {
    let fixture = fixture(super::Scenario::History(Scenario::SlowPages)).await;
    let mut client = fixture.client.clone();
    let mut progress = Vec::new();
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        snapshot(&mut client, fixture.session, RPC_TIMEOUT, |loaded| {
            progress.push(loaded);
        }),
    )
    .await
    .expect("successful paginated history is bounded")
    .expect("each page finishes within its own deadline");
    assert!(started.elapsed() > RPC_TIMEOUT);
    assert_eq!(progress, vec![50, 100, 150, 200, 250, 300]);
    assert_eq!(result.session.meta.id, fixture.session);
    assert_eq!(
        result
            .events
            .iter()
            .map(|event| event.cursor.raw())
            .collect::<Vec<_>>(),
        (1..=300).collect::<Vec<_>>()
    );
    assert_eq!(
        *fixture
            .history_requests
            .lock()
            .expect("history request log"),
        vec![None, Some(50), Some(100), Some(150), Some(200), Some(250)]
    );
}

async fn assert_hung_rpc(scenario: Scenario, expected_progress: Vec<usize>) {
    let fixture = fixture(super::Scenario::History(scenario)).await;
    let mut client = fixture.client.clone();
    let mut progress = Vec::new();
    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        snapshot(&mut client, fixture.session, RPC_TIMEOUT, |loaded| {
            progress.push(loaded);
        }),
    )
    .await
    .expect("a hung RPC must not hang the entire snapshot");
    assert!(matches!(result, Err(Failure::Timeout)));
    assert_eq!(progress, expected_progress);
    assert_eq!(
        *fixture
            .history_requests
            .lock()
            .expect("history request log"),
        vec![None, Some(256)]
    );
}

#[tokio::test]
async fn hung_later_page_times_out_without_restarting_history() {
    assert_hung_rpc(Scenario::HungPage, vec![256]).await;
}

#[tokio::test]
async fn hung_metadata_times_out_after_all_pages_report_progress() {
    assert_hung_rpc(Scenario::HungMetadata, vec![256, 300]).await;
}

#[tokio::test]
async fn repeated_and_backward_cursors_stop_without_requesting_another_page() {
    for scenario in [Scenario::RepeatedCursor, Scenario::BackwardCursor] {
        let fixture = fixture(super::Scenario::History(scenario)).await;
        let mut progress = Vec::new();
        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            snapshot(
                &mut fixture.client.clone(),
                fixture.session,
                RPC_TIMEOUT,
                |loaded| {
                    progress.push(loaded);
                },
            ),
        )
        .await
        .expect("invalid cursor terminates snapshot");
        assert!(matches!(
            result,
            Err(Failure::Condition(Condition::RepeatedHistoryCursor))
        ));
        assert_eq!(progress, vec![256]);
        assert_eq!(
            *fixture
                .history_requests
                .lock()
                .expect("history request log"),
            vec![None, Some(256)]
        );
    }
}

#[tokio::test]
async fn invalid_payload_and_wrong_session_stop_the_load() {
    for scenario in [Scenario::InvalidPayload, Scenario::WrongSession] {
        let fixture = fixture(super::Scenario::History(scenario)).await;
        let mut progress = Vec::new();
        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            snapshot(
                &mut fixture.client.clone(),
                fixture.session,
                RPC_TIMEOUT,
                |loaded| {
                    progress.push(loaded);
                },
            ),
        )
        .await
        .expect("invalid payload terminates snapshot");
        assert!(matches!(result, Err(Failure::InvalidPayload(_))));
        assert_eq!(progress, vec![256]);
        assert_eq!(
            *fixture
                .history_requests
                .lock()
                .expect("history request log"),
            vec![None, Some(256)]
        );
    }
}

#[tokio::test]
async fn watch_reports_progress_then_failed_load_without_attaching_or_retrying() {
    let fixture = fixture(super::Scenario::History(Scenario::Unavailable)).await;
    // Unlike NotFound, Unavailable previously restarted page one forever.
    // Await channel closure as well as the error to prove the watch stops.
    let updates = tokio::time::timeout(TEST_TIMEOUT, collect(&fixture))
        .await
        .expect("watch terminates after a failed snapshot");
    assert_eq!(updates.len(), 3);
    assert!(matches!(
        updates[0],
        Update::Stream {
            epoch: 1,
            state: StreamState::Loading
        }
    ));
    assert!(matches!(
        updates[1],
        Update::HistoryProgress {
            epoch: 1,
            loaded: 256
        }
    ));
    assert!(matches!(
        updates[2],
        Update::Stream {
            epoch: 1,
            state: StreamState::Unavailable(_)
        }
    ));
    assert!(
        fixture
            .attachments
            .lock()
            .expect("attachment log")
            .is_empty()
    );
    assert_eq!(
        *fixture
            .history_requests
            .lock()
            .expect("history request log"),
        vec![None, Some(256)]
    );
}
