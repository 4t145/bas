use super::*;
use prost::Message;

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
