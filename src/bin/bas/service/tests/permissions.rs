use super::*;
use bas::{acp, model::PendingPermission};

fn approval_request(session: bas::model::SessionId) -> acp::RequestPermissionRequest {
    acp::RequestPermissionRequest::new(
        session.to_string(),
        acp::ToolCallUpdate::new(
            "call",
            acp::ToolCallUpdateFields::new().title("run command"),
        ),
        vec![acp::PermissionOption::new(
            "allow",
            "Allow once",
            acp::PermissionOptionKind::AllowOnce,
        )],
    )
}

fn response(
    session: &crate::session::Session,
    request_id: &str,
    option: &str,
) -> pb::RespondPermissionRequest {
    pb::RespondPermissionRequest {
        session: session.id().to_string(),
        request_id: request_id.to_owned(),
        response: Some(
            (&acp::RequestPermissionResponse::new(acp::RequestPermissionOutcome::Selected(
                acp::SelectedPermissionOutcome::new(option.to_owned()),
            )))
                .into(),
        ),
    }
}

#[tokio::test]
async fn approval_rpc_validates_options_and_publishes_authoritative_snapshots() {
    let registry = registry(100);
    let session = create(&registry).await;
    let service = Service::new(registry.clone());
    let mut stream = service
        .watch_permissions(Request::new(pb::WatchPermissionsRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("approval subscription")
        .into_inner();
    assert!(
        stream
            .next()
            .await
            .expect("first snapshot")
            .expect("valid snapshot")
            .pending
            .is_empty()
    );
    let host = registry.permissions().host(Arc::new(LocalHost::default()));
    let id = session.id();
    let answer = tokio::spawn(async move {
        host.request_permission(acp::RequestPermissionRequest::new(
            id.to_string(),
            acp::ToolCallUpdate::new("call", acp::ToolCallUpdateFields::new().title("write file")),
            vec![acp::PermissionOption::new(
                "allow",
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            )],
        ))
        .await
        .expect("approval outcome")
    });
    let snapshot = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("approval deadline")
        .expect("approval update")
        .expect("valid snapshot");
    let pending: PendingPermission = snapshot
        .pending
        .into_iter()
        .next()
        .expect("pending request")
        .try_into()
        .expect("canonical ACP request");
    assert_eq!(
        pending.request.session_id.to_string(),
        session.id().to_string()
    );
    let invalid = service
        .respond_permission(Request::new(response(
            &session,
            &pending.request_id,
            "invented",
        )))
        .await
        .expect_err("unknown option");
    assert_eq!(invalid.code(), Code::InvalidArgument);
    assert!(!answer.is_finished());
    service
        .respond_permission(Request::new(response(
            &session,
            &pending.request_id,
            "allow",
        )))
        .await
        .expect("offered option");
    assert_eq!(
        answer.await.expect("request task"),
        acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new("allow"))
    );
    assert!(
        stream
            .next()
            .await
            .expect("resolved snapshot")
            .expect("valid snapshot")
            .pending
            .is_empty()
    );
    let expired = service
        .respond_permission(Request::new(response(
            &session,
            &pending.request_id,
            "allow",
        )))
        .await
        .expect_err("approval already answered");
    assert_eq!(expired.code(), Code::FailedPrecondition);
    drop(stream);
    registry.shutdown().await;
}

#[tokio::test]
async fn approval_subscription_closes_when_daemon_stops() {
    let registry = registry(100);
    let session = create(&registry).await;
    let service = Service::new(registry.clone());
    let mut stream = service
        .watch_permissions(Request::new(pb::WatchPermissionsRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("approval subscription")
        .into_inner();
    stream
        .next()
        .await
        .expect("first snapshot")
        .expect("valid snapshot");
    registry.shutdown().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("shutdown releases approval stream")
            .is_none()
    );
}

#[tokio::test]
async fn disconnecting_real_grpc_approval_stream_cancels_pending_request() {
    use crate::agent::PermissionPolicy;
    use tokio_stream::wrappers::TcpListenerStream;
    let registry = registry(100);
    let session = create(&registry).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(Service::new(registry.clone()).into_server())
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let mut client = bas::service::Client::connect_at(address)
        .await
        .expect("client");
    let mut stream = client
        .watch_permissions(pb::WatchPermissionsRequest {
            session: session.id().to_string(),
        })
        .await
        .expect("approval subscription")
        .into_inner();
    stream
        .message()
        .await
        .expect("snapshot")
        .expect("first snapshot");
    let host = registry
        .permissions()
        .host(Arc::new(LocalHost::new(PermissionPolicy::Allow)));
    let id = session.id();
    let answer = tokio::spawn(async move { host.request_permission(approval_request(id)).await });
    let snapshot = tokio::time::timeout(Duration::from_secs(2), stream.message())
        .await
        .expect("approval deadline")
        .expect("valid snapshot")
        .expect("pending approval");
    assert_eq!(snapshot.pending.len(), 1);
    assert!(!answer.is_finished());
    drop(stream);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), answer)
            .await
            .expect("disconnect cancels outstanding approval")
            .expect("request task")
            .expect("approval outcome"),
        acp::RequestPermissionOutcome::Cancelled
    );
    registry.shutdown().await;
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn deleting_session_closes_approval_stream_and_cancels_pending_request() {
    let registry = registry(100);
    let session = create(&registry).await;
    let service = Service::new(registry.clone());
    let mut stream = service
        .watch_permissions(Request::new(pb::WatchPermissionsRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("approval subscription")
        .into_inner();
    stream
        .next()
        .await
        .expect("first snapshot")
        .expect("valid snapshot");
    let host = registry.permissions().host(Arc::new(LocalHost::default()));
    let id = session.id();
    let answer = tokio::spawn(async move { host.request_permission(approval_request(id)).await });
    let snapshot = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("approval deadline")
        .expect("approval update")
        .expect("valid snapshot");
    assert_eq!(snapshot.pending.len(), 1);
    registry.remove(id).await.expect("delete session");
    assert_eq!(
        answer
            .await
            .expect("request task")
            .expect("approval outcome"),
        acp::RequestPermissionOutcome::Cancelled
    );
    if let Some(snapshot) = stream.next().await {
        assert!(snapshot.expect("final snapshot").pending.is_empty());
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("removed session stream closes")
            .is_none()
    );
    registry.shutdown().await;
}
