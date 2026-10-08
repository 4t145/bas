use super::*;

async fn pending_update(events: &mut mpsc::Receiver<Update>) -> Vec<PendingPermission> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Update::Permissions { epoch, pending } =
                events.recv().await.expect("approval watcher stays alive")
            {
                assert_eq!(epoch, 9);
                return pending;
            }
        }
    })
    .await
    .expect("approval snapshot arrives over gRPC")
}

async fn unavailable_update(events: &mut mpsc::Receiver<Update>) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("watcher stays alive") {
                Update::PermissionUnavailable { epoch, message } => {
                    assert_eq!(epoch, 9);
                    return message;
                }
                Update::Permissions { .. } => {
                    panic!("approval failure must not publish a successful snapshot");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("approval failure is visible")
}

#[tokio::test]
async fn disconnected_approval_stream_reports_failure_without_a_successful_empty_snapshot() {
    let fixture = fixture(Scenario::ApprovalDisconnected).await;
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(crate::connection::permissions::watch_session(
        fixture.client.clone(),
        updates,
        fixture.session,
        9,
    ));
    assert_eq!(pending_update(&mut events).await.len(), 1);
    let message = unavailable_update(&mut events).await;
    assert!(message.contains("审批连接中断"));
    assert!(message.contains("待审批请求已取消"));
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn legacy_daemon_reports_upgrade_required_for_interactive_approvals() {
    let fixture = fixture(Scenario::LegacyApprovals).await;
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(crate::connection::permissions::watch_session(
        fixture.client.clone(),
        updates,
        fixture.session,
        9,
    ));
    let message = unavailable_update(&mut events).await;
    assert!(message.contains("升级并重启"));
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn subscription_and_response_preserve_native_choices_over_grpc() {
    let fixture = fixture(Scenario::Approval).await;
    let (updates, mut events) = mpsc::channel(EVENT_CAPACITY);
    let task = tokio::spawn(crate::connection::permissions::watch_session(
        fixture.client.clone(),
        updates,
        fixture.session,
        9,
    ));
    let pending = pending_update(&mut events).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].request.tool_call.fields.raw_input,
        Some(serde_json::json!({"command": "pwd"}))
    );
    let request_id = pending[0].request_id.clone();
    let invalid = execute(
        fixture.client.clone(),
        Command::RespondPermission {
            session: fixture.session,
            request_id: request_id.clone(),
            outcome: acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                "invented",
            )),
        },
    )
    .await;
    assert!(matches!(
        invalid,
        Update::PermissionResponded { result: Err(_), .. }
    ));
    assert!(
        fixture
            .permission_answers
            .lock()
            .expect("answers")
            .is_empty()
    );
    let answered = execute(
        fixture.client.clone(),
        Command::RespondPermission {
            session: fixture.session,
            request_id: request_id.clone(),
            outcome: acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                "allow-once",
            )),
        },
    )
    .await;
    assert!(
        matches!(answered, Update::PermissionResponded { session, request_id: id, result: Ok(()) }
        if session == fixture.session && id == request_id)
    );
    assert!(pending_update(&mut events).await.is_empty());
    let duplicate = execute(
        fixture.client.clone(),
        Command::RespondPermission {
            session: fixture.session,
            request_id,
            outcome: acp::RequestPermissionOutcome::Cancelled,
        },
    )
    .await;
    assert!(matches!(
        duplicate,
        Update::PermissionResponded { result: Err(_), .. }
    ));
    assert_eq!(fixture.permission_answers.lock().expect("answers").len(), 1);
    task.abort();
    let _ = task.await;
}
