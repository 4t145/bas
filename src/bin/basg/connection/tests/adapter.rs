use super::*;

#[tokio::test]
async fn request_retains_rpc_code_and_details_through_anyhow_context() {
    let details = vec![0, 1, 255, 7];
    let status = Status::with_details(
        Code::PermissionDenied,
        "daemon diagnostic",
        details.clone().into(),
    );
    let failure =
        request::<()>(async { Err(anyhow::Error::from(status).context("external client")) })
            .await
            .expect_err("failed request");
    assert!(
        matches!(failure, Failure::Rpc { code: Code::PermissionDenied, ref message, details: ref actual, .. }
        if message == "daemon diagnostic" && actual == &details)
    );
    assert!(failure.presentation().contains("daemon diagnostic"));
}

#[test]
fn pending_entries_decode_independently_and_keep_identity_and_answered_state() {
    let raw = serde_json::json!({
        "mode": "form", "message": "Configure", "requestId": 7,
        "requestedSchema": {"type": "object", "properties": {
            "name": {"type": "string"}
        }}
    });
    let entries: Vec<PendingElicitation> = vec![
        pb::PendingElicitation {
            request_id: "valid".into(),
            answered: true,
            request: Some(pb::AcpPayload {
                json: serde_json::to_vec(&raw).expect("JSON"),
            }),
        },
        pb::PendingElicitation {
            request_id: "invalid".into(),
            answered: false,
            request: Some(pb::AcpPayload {
                json: b"not json".to_vec(),
            }),
        },
        pb::PendingElicitation {
            request_id: "missing".into(),
            answered: false,
            request: None,
        },
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    assert_eq!(entries[0].request_id, "valid");
    assert!(entries[0].answered);
    let request = entries[0].request.as_ref().expect("valid domain request");
    assert_eq!(request.message, "Configure");
    assert_eq!(request.raw, raw);
    assert!(matches!(
        entries[1].request,
        Err(Failure::InvalidPayload(_))
    ));
    assert!(matches!(
        entries[2].request,
        Err(Failure::Condition(Condition::MissingElicitationRequest))
    ));
    assert_eq!(
        entries[2]
            .request
            .as_ref()
            .expect_err("missing request")
            .presentation(),
        crate::i18n::tr!("elicitation-empty")
    );
    assert_eq!(entries[1].request_id, "invalid");
}
