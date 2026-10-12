use super::*;
use prost::Message;

#[test]
fn submission_envelope_preserves_identity_and_native_payload() {
    let request = acp::PromptRequest::new(SessionId::new().to_string(), vec!["prompt".into()])
        .meta(
            serde_json::from_value::<acp::Meta>(
                serde_json::json!({"large": 9_007_199_254_740_993_u64, "nested": {"key": [1, 2]}}),
            )
            .expect("metadata"),
        );
    let submission = PromptSubmission::new(request.clone());
    let next = PromptSubmission::new(request);
    assert_ne!(submission.turn, next.turn);
    let bytes = pb::PromptSubmission::from(&submission).encode_to_vec();
    let decoded = PromptSubmission::try_from(
        pb::PromptSubmission::decode(bytes.as_slice()).expect("protobuf"),
    )
    .expect("submission");
    assert_eq!(decoded, submission);
    assert_eq!(
        pb::PromptSubmission::from(&decoded).encode_to_vec(),
        bytes,
        "retry preserves the original identity and payload"
    );
}

#[test]
fn malformed_submission_envelopes_do_not_become_prompts() {
    let submission = PromptSubmission::new(acp::PromptRequest::new(
        SessionId::new().to_string(),
        vec!["prompt".into()],
    ));
    let mut wire = pb::PromptSubmission::from(&submission);
    wire.turn = "invalid-turn".into();
    assert!(matches!(
        PromptSubmission::try_from(wire),
        Err(Error::Id(_))
    ));
    let mut wire = pb::PromptSubmission::from(&submission);
    wire.prompt = None;
    assert!(matches!(
        PromptSubmission::try_from(wire),
        Err(Error::MissingPrompt)
    ));
    let mut wire = pb::PromptSubmission::from(&submission);
    wire.prompt = Some(pb::AcpPayload {
        json: b"{}".to_vec(),
    });
    assert!(matches!(
        PromptSubmission::try_from(wire),
        Err(Error::Json(_))
    ));
}

#[test]
fn acceptance_and_explicit_rejections_round_trip_separately_from_acp_results() {
    let mut outcomes = vec![PromptSubmissionOutcome::Accepted(TurnReceipt {
        turn: TurnId::new(),
        cursor: Cursor::from_raw(123),
    })];
    outcomes.extend(
        [
            PromptRejectionKind::InvalidRequest,
            PromptRejectionKind::Busy,
            PromptRejectionKind::Archived,
            PromptRejectionKind::Unavailable,
            PromptRejectionKind::Conflict,
        ]
        .map(|kind| {
            PromptSubmissionOutcome::Rejected(PromptRejection {
                kind,
                message: "explicit refusal".into(),
            })
        }),
    );
    for outcome in outcomes {
        let bytes = pb::PromptSubmissionResponse::from(outcome.clone()).encode_to_vec();
        let decoded = PromptSubmissionOutcome::try_from(
            pb::PromptSubmissionResponse::decode(bytes.as_slice()).expect("protobuf"),
        )
        .expect("outcome");
        assert_eq!(decoded, outcome);
    }
}

#[test]
fn malformed_responses_never_imply_acceptance_or_rejection() {
    assert!(matches!(
        PromptSubmissionOutcome::try_from(pb::PromptSubmissionResponse { outcome: None }),
        Err(Error::MissingSubmissionOutcome)
    ));
    for kind in [pb::PromptRejectionKind::Unspecified as i32, 999] {
        let wire = pb::PromptSubmissionResponse {
            outcome: Some(pb::prompt_submission_response::Outcome::Rejected(
                pb::PromptRejection {
                    kind,
                    message: "unrecognized refusal".into(),
                },
            )),
        };
        assert!(matches!(
            PromptSubmissionOutcome::try_from(wire),
            Err(Error::Unknown {
                field: "prompt rejection kind",
                ..
            })
        ));
    }
    let wire = pb::PromptSubmissionResponse {
        outcome: Some(pb::prompt_submission_response::Outcome::Accepted(
            pb::TurnAccepted {
                turn: TurnId::new().to_string(),
                cursor: 0,
            },
        )),
    };
    assert!(matches!(
        PromptSubmissionOutcome::try_from(wire),
        Err(Error::InvalidReceipt)
    ));
}

#[test]
fn receipts_require_a_valid_identity_and_committed_cursor() {
    let turn = TurnId::new();
    let receipt = TurnReceipt {
        turn,
        cursor: Cursor::from_raw(123),
    };
    let wire = pb::TurnAccepted::from(receipt);
    assert_eq!(wire.turn, turn.to_string());
    assert_eq!(wire.cursor, 123);
    assert_eq!(TurnReceipt::try_from(wire).expect("receipt"), receipt);
    assert!(matches!(
        TurnReceipt::try_from(pb::TurnAccepted {
            turn: turn.to_string(),
            ..Default::default()
        }),
        Err(Error::InvalidReceipt)
    ));
    assert!(matches!(
        TurnReceipt::try_from(pb::TurnAccepted {
            turn: "invalid-turn".into(),
            cursor: 123,
        }),
        Err(Error::Id(_))
    ));
}
