use super::*;

#[derive(Clone, Copy)]
pub(super) enum Scenario {
    Finished,
    Interrupted,
    Reconnect,
    Missing,
    Expired,
    Denied,
    Invalid,
    Malformed,
    Eof,
    Pending,
}

pub(super) fn attach(
    service: &Service,
    scenario: Scenario,
    count: usize,
) -> Result<Response<<Service as Bas>::AttachStream>, Status> {
    let event =
        |cursor, kind| Ok((&Event::new(service.session.id, Cursor::from_raw(cursor), kind)).into());
    let finished = |turn| EventKind::TurnFinished {
        turn,
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
    };
    let interrupted = |turn| EventKind::TurnInterrupted {
        turn,
        reason: InterruptionReason::DaemonRestarted,
    };
    let other = TurnId::new();
    let events = match scenario {
        Scenario::Finished => vec![
            event(8, interrupted(other)),
            event(9, finished(service.turn)),
            event(10, interrupted(other)),
        ],
        Scenario::Interrupted => vec![
            event(8, finished(other)),
            event(9, interrupted(service.turn)),
            event(10, finished(other)),
        ],
        Scenario::Reconnect if count == 1 => vec![
            event(8, finished(other)),
            Err(Status::unavailable("transport disconnected")),
        ],
        Scenario::Reconnect => vec![event(9, finished(service.turn))],
        Scenario::Missing => return Err(Status::not_found("session deleted")),
        Scenario::Expired => return Err(Status::data_loss("cursor evicted")),
        Scenario::Denied => return Err(Status::unauthenticated("access denied")),
        Scenario::Invalid => return Err(Status::invalid_argument("invalid cursor")),
        Scenario::Malformed => vec![Ok(pb::Event::default())],
        Scenario::Eof => vec![event(8, finished(other))],
        Scenario::Pending => return Ok(Response::new(Box::pin(futures::stream::pending()))),
    };
    Ok(Response::new(Box::pin(tokio_stream::iter(events))))
}

fn prompt(session: SessionId, turn: TurnId) -> Command {
    Command::Prompt {
        session,
        submission: crate::submission::prepare(
            session,
            turn,
            "accepted",
            &[],
            &acp::PromptCapabilities::default(),
        ),
        title: None,
    }
}

async fn operation_updates(scenario: Scenario) -> (Fixture, Vec<Update>) {
    let fixture = fixture(super::Scenario::Tracking(scenario)).await;
    let (updates, mut events) = mpsc::channel(8);
    operation(
        fixture.client.clone(),
        prompt(fixture.session, fixture.turn),
        updates,
        Arc::new(Notify::new()),
    )
    .await;
    let mut received = Vec::new();
    while let Some(update) = events.recv().await {
        received.push(update);
    }
    (fixture, received)
}

#[tokio::test]
async fn sent_precedes_authoritative_outcome_and_other_turns_cannot_replace_it() {
    for scenario in [Scenario::Finished, Scenario::Interrupted] {
        let (fixture, updates) = operation_updates(scenario).await;
        assert_eq!(updates.len(), 2);
        assert!(
            matches!(&updates[0], Update::Submission { result: Ok(PromptSubmissionOutcome::Accepted(receipt)), .. }
            if receipt.turn == fixture.turn && receipt.cursor.raw() == 7)
        );
        let Update::TrackedTurn { event } = &updates[1] else {
            panic!("expected authoritative tracked result");
        };
        assert_eq!(event.session, fixture.session);
        assert_eq!(event.cursor.raw(), 9);
        match scenario {
            Scenario::Finished => assert!(
                matches!(event.kind, EventKind::TurnFinished { turn, .. } if turn == fixture.turn)
            ),
            Scenario::Interrupted => assert!(
                matches!(event.kind, EventKind::TurnInterrupted { turn, .. } if turn == fixture.turn)
            ),
            _ => unreachable!(),
        }
        assert_eq!(*fixture.attachments.lock().expect("attach log"), [Some(7)]);
    }
}

#[tokio::test]
async fn transport_reconnect_resumes_at_last_decoded_cursor() {
    let (fixture, updates) = operation_updates(Scenario::Reconnect).await;
    assert!(matches!(&updates[1], Update::TrackedTurn { event }
        if matches!(event.kind, EventKind::TurnFinished { turn, .. } if turn == fixture.turn)));
    assert_eq!(
        *fixture.attachments.lock().expect("attach log"),
        [Some(7), Some(8)]
    );
}

#[tokio::test]
async fn unavailable_history_and_incomplete_streams_fail_only_the_accepted_receipt() {
    for scenario in [
        Scenario::Missing,
        Scenario::Expired,
        Scenario::Denied,
        Scenario::Invalid,
        Scenario::Malformed,
        Scenario::Eof,
    ] {
        let (fixture, updates) = operation_updates(scenario).await;
        assert_eq!(updates.len(), 2);
        assert!(matches!(
            &updates[0],
            Update::Submission {
                result: Ok(PromptSubmissionOutcome::Accepted(_)),
                ..
            }
        ));
        assert!(
            matches!(&updates[1], Update::TrackingFailed { session, receipt, message }
            if *session == fixture.session && receipt.turn == fixture.turn && receipt.cursor.raw() == 7 && !message.presentation().is_empty())
        );
        assert_eq!(*fixture.attachments.lock().expect("attach log"), [Some(7)]);
    }
}

#[tokio::test]
async fn dropping_connection_aborts_pending_turn_tracking() {
    let fixture = fixture(super::Scenario::Tracking(Scenario::Pending)).await;
    let (commands, receiver) = mpsc::unbounded_channel();
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let connection = Connection {
        commands,
        task: Some(tokio::spawn(run(fixture.client.clone(), receiver, updates))),
    };
    assert!(
        connection
            .send(prompt(fixture.session, fixture.turn))
            .is_ok()
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.attachments.lock().expect("attach log").is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("tracking attached");
    drop(connection);
    tokio::time::timeout(Duration::from_secs(5), async {
        while events.recv().await.is_some() {}
    })
    .await
    .expect("all operation tasks dropped their update senders");
}
