use super::*;

#[derive(Clone, Copy)]
pub(super) enum Scenario {
    LostAccepted,
    LostMissing,
    QueryFailure,
    BadReceipt,
    Rejected,
    TitleFailure,
}

pub(super) fn prompt(
    service: &Service,
    scenario: Scenario,
    submission: PromptSubmission,
) -> Result<Response<pb::PromptSubmissionResponse>, Status> {
    let receipt = TurnReceipt {
        turn: submission.turn,
        cursor: Cursor::from_raw(7),
    };
    let mut calls = service.submissions.lock().expect("submissions");
    calls.push(submission);
    match scenario {
        Scenario::BadReceipt => Ok(Response::new(
            PromptSubmissionOutcome::Accepted(TurnReceipt {
                cursor: Cursor::START,
                ..receipt
            })
            .into(),
        )),
        Scenario::Rejected => Ok(Response::new(
            PromptSubmissionOutcome::Rejected(PromptRejection {
                kind: PromptRejectionKind::Busy,
                message: "busy".into(),
            })
            .into(),
        )),
        _ if calls.len() == 1 => Err(Status::unavailable("accepted but response lost")),
        _ => Ok(Response::new(
            PromptSubmissionOutcome::Accepted(receipt).into(),
        )),
    }
}

pub(super) fn acceptance(
    service: &Service,
    scenario: Scenario,
    request: pb::PromptAcceptanceRequest,
) -> Result<Response<pb::PromptAcceptanceResponse>, Status> {
    assert_eq!(request.session, service.session.id.to_string());
    assert_eq!(request.turn, service.turn.to_string());
    if matches!(scenario, Scenario::QueryFailure) {
        return Err(Status::unavailable("query disconnected"));
    }
    Ok(Response::new(pb::PromptAcceptanceResponse {
        accepted: matches!(scenario, Scenario::LostAccepted).then(|| pb::TurnAccepted {
            turn: request.turn,
            cursor: 7,
        }),
    }))
}

fn initial(fixture: &Fixture) -> Command {
    Command::Prompt {
        session: fixture.session,
        submission: crate::submission::prepare(
            fixture.session,
            fixture.turn,
            "original payload",
            &[],
            &acp::PromptCapabilities::default(),
        ),
        title: Some("initial title".into()),
    }
}

#[tokio::test]
async fn lost_response_queries_do_not_resubmit_and_acceptance_starts_background_tracking() {
    for scenario in [
        Scenario::LostAccepted,
        Scenario::LostMissing,
        Scenario::QueryFailure,
    ] {
        let fixture = fixture(super::Scenario::Submission(scenario)).await;
        let Update::Submission {
            submission,
            result: Err(_),
            ..
        } = execute(fixture.client.clone(), initial(&fixture)).await
        else {
            panic!("lost response must remain unknown");
        };
        let (updates, mut events) = mpsc::channel(8);
        operation(
            fixture.client.clone(),
            Command::ResolvePrompt {
                session: fixture.session,
                submission,
                retry: false,
            },
            updates,
            Arc::new(Notify::new()),
        )
        .await;
        let first = events.recv().await.expect("query result");
        if matches!(scenario, Scenario::LostAccepted) {
            assert!(matches!(
                first,
                Update::Submission {
                    result: Ok(PromptSubmissionOutcome::Accepted(_)),
                    ..
                }
            ));
            assert!(
                matches!(events.recv().await, Some(Update::TrackedTurn { event })
                if matches!(event.kind, EventKind::TurnFinished { turn, .. } if turn == fixture.turn))
            );
            assert_eq!(
                *fixture.attachments.lock().expect("tracking log"),
                [Some(7)]
            );
        } else {
            assert!(matches!(first, Update::Submission { result: Err(_), .. }));
            assert!(fixture.attachments.lock().expect("tracking log").is_empty());
        }
        assert!(events.recv().await.is_none());
        assert_eq!(fixture.submissions.lock().expect("submissions").len(), 1);
    }
}

#[tokio::test]
async fn explicit_retry_reuses_original_identity_payload_and_does_not_restore_old_title() {
    let fixture = fixture(super::Scenario::Submission(Scenario::LostAccepted)).await;
    let Update::Submission {
        submission,
        result: Err(_),
        ..
    } = execute(fixture.client.clone(), initial(&fixture)).await
    else {
        panic!("unknown");
    };
    execute(
        fixture.client.clone(),
        Command::Rename {
            session: fixture.session,
            title: "later title".into(),
        },
    )
    .await;
    let update = execute(
        fixture.client.clone(),
        Command::ResolvePrompt {
            session: fixture.session,
            submission: submission.clone(),
            retry: true,
        },
    )
    .await;
    assert!(matches!(
        update,
        Update::Submission {
            result: Ok(PromptSubmissionOutcome::Accepted(_)),
            ..
        }
    ));
    assert_eq!(
        *fixture.submissions.lock().expect("submissions"),
        [submission.as_ref().clone(), submission.as_ref().clone()]
    );
    assert_eq!(
        *fixture.titles.lock().expect("titles"),
        ["initial title", "later title"]
    );
}

#[tokio::test]
async fn invalid_receipt_is_unknown_but_preflight_and_explicit_rejections_are_definitive() {
    for scenario in [
        Scenario::BadReceipt,
        Scenario::TitleFailure,
        Scenario::Rejected,
    ] {
        let fixture = fixture(super::Scenario::Submission(scenario)).await;
        let update = execute(fixture.client.clone(), initial(&fixture)).await;
        if matches!(scenario, Scenario::BadReceipt) {
            assert!(matches!(update, Update::Submission { result: Err(_), .. }));
        } else if matches!(scenario, Scenario::TitleFailure) {
            assert!(matches!(
                update,
                Update::Submission {
                    result: Err(Failure::Preflight(_)),
                    ..
                }
            ));
        } else {
            assert!(matches!(
                update,
                Update::Submission {
                    result: Ok(PromptSubmissionOutcome::Rejected(_)),
                    ..
                }
            ));
        }
        assert_eq!(
            fixture.submissions.lock().expect("submissions").len(),
            if matches!(scenario, Scenario::TitleFailure) {
                0
            } else {
                1
            }
        );
    }
}
