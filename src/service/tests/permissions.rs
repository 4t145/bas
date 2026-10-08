use super::*;

#[test]
fn approval_payload_preserves_tool_details_choices_and_extension_metadata() {
    let request = acp::RequestPermissionRequest::new(
        SessionId::new().to_string(),
        acp::ToolCallUpdate::new(
            "tool",
            acp::ToolCallUpdateFields::new()
                .title("run command")
                .raw_input(
                    serde_json::json!({"command": "ls /tmp", "large": 9007199254740993_u64}),
                ),
        ),
        vec![acp::PermissionOption::new(
            "custom-id",
            "Allow this command",
            acp::PermissionOptionKind::AllowOnce,
        )],
    )
    .meta(
        serde_json::from_value::<acp::Meta>(serde_json::json!({"extension": [1,2,3]}))
            .expect("metadata"),
    );
    let pending = PendingPermission {
        request_id: "daemon-request-id".into(),
        request,
    };
    assert_eq!(
        PendingPermission::try_from(pb::PendingPermission::from(&pending))
            .expect("approval roundtrip"),
        pending
    );
    let outcome = acp::RequestPermissionResponse::new(acp::RequestPermissionOutcome::Selected(
        acp::SelectedPermissionOutcome::new("custom-id"),
    ));
    assert_eq!(
        pb::AcpPayload::from(&outcome)
            .decode::<acp::RequestPermissionResponse>()
            .expect("outcome roundtrip"),
        outcome
    );
    assert!(matches!(
        PendingPermission::try_from(pb::PendingPermission {
            request_id: "id".into(),
            request: None
        }),
        Err(Error::MissingPermissionRequest)
    ));
}
