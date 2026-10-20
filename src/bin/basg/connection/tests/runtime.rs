use super::*;

#[tokio::test]
async fn failed_snapshot_delivers_history_before_unavailable_without_attaching() {
    let fixture = fixture(Scenario::RuntimeFailure).await;
    let updates = collect(&fixture).await;
    assert!(matches!(&updates[..], [
        Update::Stream { state: StreamState::Loading, .. },
        Update::HistoryProgress { loaded: 256, .. },
        Update::HistoryProgress { loaded: 300, .. },
        Update::Snapshot { session, events, .. },
        Update::Stream { state: StreamState::Unavailable(message), .. },
    ] if session.runtime.failure.is_some() && events.len() == 300 && message.presentation().contains("disk full")));
    assert!(fixture.attachments.lock().expect("attachments").is_empty());
}

#[tokio::test]
async fn failed_precondition_is_terminal_on_attach_and_stream() {
    for scenario in [
        Scenario::AttachFailedPrecondition,
        Scenario::StreamFailedPrecondition,
    ] {
        let fixture = fixture(scenario).await;
        let updates = collect(&fixture).await;
        assert!(
            matches!(updates.last(), Some(Update::Stream { state: StreamState::Unavailable(message), .. }) if message.presentation().contains("disk full"))
        );
        assert!(!updates.iter().any(|update| matches!(
            update,
            Update::Stream {
                state: StreamState::Reconnecting(_),
                ..
            }
        )));
        assert_eq!(fixture.attachments.lock().expect("attachments").len(), 1);
        assert!(
            !updates
                .iter()
                .any(|update| matches!(update, Update::Event { .. }))
        );
    }
}
