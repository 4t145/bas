use super::*;
use crate::agent::{LocalHost, PermissionPolicy};

fn request(session: SessionId) -> acp::RequestPermissionRequest {
    acp::RequestPermissionRequest::new(
        session.to_string(),
        acp::ToolCallUpdate::new(
            "tool-1",
            acp::ToolCallUpdateFields::new().title("run a command"),
        ),
        vec![
            acp::PermissionOption::new(
                "allow-once",
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                "reject-once",
                "Reject once",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ],
    )
}

fn selected(id: &str) -> acp::RequestPermissionOutcome {
    acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(id.to_owned()))
}

async fn next_permission(subscription: &mut PermissionSubscription) -> PendingPermission {
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        subscription.snapshots.changed(),
    )
    .await
    .expect("approval snapshot arrives")
    .expect("subscription stays open");
    subscription
        .snapshots
        .borrow_and_update()
        .first()
        .expect("pending approval")
        .clone()
}

#[tokio::test]
async fn headless_requests_keep_configured_policy() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    for (policy, expected) in [
        (PermissionPolicy::Allow, "allow-once"),
        (PermissionPolicy::Deny, "reject-once"),
    ] {
        let host = permissions.host(Arc::new(LocalHost::new(policy)));
        let outcome = host
            .request_permission(request(session))
            .await
            .expect("headless policy");
        assert_eq!(outcome, selected(expected));
    }
}

#[tokio::test]
async fn attached_client_must_answer_even_with_allow_policy() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    let mut subscription = permissions.subscribe(session);
    let host = permissions.host(Arc::new(LocalHost::new(PermissionPolicy::Allow)));
    let answer = tokio::spawn(async move { host.request_permission(request(session)).await });
    let pending = next_permission(&mut subscription).await;
    assert!(
        !answer.is_finished(),
        "interactive requests wait for the user"
    );
    permissions
        .respond(session, &pending.request_id, selected("reject-once"))
        .expect("valid choice");
    assert_eq!(
        answer
            .await
            .expect("request task")
            .expect("approval outcome"),
        selected("reject-once")
    );
    assert!(subscription.snapshots.borrow().is_empty());
    assert!(matches!(
        permissions.respond(session, &pending.request_id, selected("allow-once")),
        Err(PermissionError::Expired)
    ));
}

#[tokio::test]
async fn invalid_options_and_wrong_sessions_preserve_pending_request() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    let mut subscription = permissions.subscribe(session);
    let host = permissions.host(Arc::new(LocalHost::default()));
    let answer = tokio::spawn(async move { host.request_permission(request(session)).await });
    let pending = next_permission(&mut subscription).await;
    assert!(matches!(
        permissions.respond(session, &pending.request_id, selected("invented-option")),
        Err(PermissionError::InvalidOption)
    ));
    assert!(matches!(
        permissions.respond(
            SessionId::new(),
            &pending.request_id,
            selected("allow-once")
        ),
        Err(PermissionError::Expired)
    ));
    assert_eq!(subscription.snapshots.borrow().len(), 1);
    permissions
        .respond(session, &pending.request_id, selected("allow-once"))
        .expect("offered option remains answerable");
    assert_eq!(
        answer
            .await
            .expect("request task")
            .expect("approval outcome"),
        selected("allow-once")
    );
}

#[tokio::test]
async fn last_approver_disconnect_cancels_instead_of_falling_back_to_allow() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    let mut first = permissions.subscribe(session);
    let second = permissions.subscribe(session);
    let host = permissions.host(Arc::new(LocalHost::new(PermissionPolicy::Allow)));
    let answer = tokio::spawn(async move { host.request_permission(request(session)).await });
    let _pending = next_permission(&mut first).await;
    drop(first);
    assert!(!answer.is_finished());
    drop(second);
    assert_eq!(
        answer
            .await
            .expect("request task")
            .expect("approval outcome"),
        acp::RequestPermissionOutcome::Cancelled
    );
    assert_eq!(
        permissions
            .lock()
            .get(&session)
            .expect("interactive session stays registered")
            .viewers,
        0
    );
    let host = permissions.host(Arc::new(LocalHost::new(PermissionPolicy::Allow)));
    assert_eq!(
        host.request_permission(request(session))
            .await
            .expect("detached interactive policy"),
        acp::RequestPermissionOutcome::Cancelled
    );
}

#[tokio::test]
async fn turn_cancellation_resolves_all_pending_approvals() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    let mut subscription = permissions.subscribe(session);
    let host = permissions.host(Arc::new(LocalHost::default()));
    let answer = tokio::spawn(async move { host.request_permission(request(session)).await });
    let pending = next_permission(&mut subscription).await;
    permissions.cancel(session);
    assert_eq!(
        answer
            .await
            .expect("request task")
            .expect("approval outcome"),
        acp::RequestPermissionOutcome::Cancelled
    );
    assert!(subscription.snapshots.borrow().is_empty());
    assert!(matches!(
        permissions.respond(session, &pending.request_id, selected("allow-once")),
        Err(PermissionError::Expired)
    ));
}

#[tokio::test]
async fn aborted_backend_callback_removes_stale_approval() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    let mut subscription = permissions.subscribe(session);
    let host = permissions.host(Arc::new(LocalHost::default()));
    let answer = tokio::spawn(async move { host.request_permission(request(session)).await });
    let pending = next_permission(&mut subscription).await;
    answer.abort();
    assert!(
        answer
            .await
            .expect_err("backend request aborted")
            .is_cancelled()
    );
    assert!(subscription.snapshots.borrow().is_empty());
    assert!(matches!(
        permissions.respond(session, &pending.request_id, selected("allow-once")),
        Err(PermissionError::Expired)
    ));
}

#[tokio::test]
async fn new_viewer_receives_existing_requests_without_duplicate_decisions() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    let mut first = permissions.subscribe(session);
    let host = permissions.host(Arc::new(LocalHost::default()));
    let answer = tokio::spawn(async move { host.request_permission(request(session)).await });
    let pending = next_permission(&mut first).await;
    let second = permissions.subscribe(session);
    assert_eq!(second.snapshots.borrow().first(), Some(&pending));
    permissions
        .respond(
            session,
            &pending.request_id,
            acp::RequestPermissionOutcome::Cancelled,
        )
        .expect("cancel choice");
    assert_eq!(
        answer
            .await
            .expect("request task")
            .expect("approval outcome"),
        acp::RequestPermissionOutcome::Cancelled
    );
    assert!(first.snapshots.borrow().is_empty());
    assert!(second.snapshots.borrow().is_empty());
}

#[tokio::test]
async fn closed_subscription_cannot_detach_a_newer_channel() {
    let permissions = Arc::new(Permissions::default());
    let session = SessionId::new();
    let old = permissions.subscribe(session);
    permissions.close(session);
    let mut current = permissions.subscribe(session);
    drop(old);
    assert_eq!(
        permissions
            .lock()
            .get(&session)
            .expect("current subscription")
            .viewers,
        1
    );
    let host = permissions.host(Arc::new(LocalHost::default()));
    let answer = tokio::spawn(async move { host.request_permission(request(session)).await });
    let pending = next_permission(&mut current).await;
    permissions
        .respond(session, &pending.request_id, selected("allow-once"))
        .expect("current viewer can answer");
    assert_eq!(
        answer
            .await
            .expect("request task")
            .expect("approval outcome"),
        selected("allow-once")
    );
}
