use super::*;

fn pending() -> PendingPermission {
    PendingPermission {
        request_id: "request-1".into(),
        request: acp::RequestPermissionRequest::new(
            "session",
            acp::ToolCallUpdate::new(
                "tool-1",
                acp::ToolCallUpdateFields::new()
                    .title("run command")
                    .raw_input(serde_json::json!({"command": "ls\npwd"})),
            ),
            vec![acp::PermissionOption::new(
                "allow",
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            )],
        ),
    }
}

#[test]
fn successful_submit_waits_for_authoritative_snapshot() {
    let mut panel = Panel::default();
    panel.replace(vec![pending()]);
    panel.submitting("request-1");
    panel.responded("request-1", Ok(()));
    assert!(panel.in_flight.contains("request-1"));
    assert!(!panel.is_empty());
    panel.replace(Vec::new());
    assert!(panel.in_flight.is_empty());
    assert!(panel.is_empty());
}

#[test]
fn failures_follow_their_request_through_nonempty_snapshots_and_retries() {
    let first = pending();
    let mut second = pending();
    second.request_id = "request-2".into();
    let mut panel = Panel::default();
    panel.replace(vec![first.clone(), second.clone()]);
    panel.responded("request-1", Err("first failed".into()));
    panel.responded("request-2", Err("second failed".into()));

    panel.submitting("request-2");
    assert_eq!(
        panel.errors.get("request-1").map(String::as_str),
        Some("first failed")
    );
    assert!(!panel.errors.contains_key("request-2"));
    panel.replace(vec![second]);
    assert!(panel.errors.is_empty());
    assert!(panel.in_flight.contains("request-2"));
    panel.responded("request-1", Err("late failure".into()));
    assert!(panel.errors.is_empty());
}
