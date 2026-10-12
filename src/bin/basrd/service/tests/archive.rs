use super::*;
use basr::{acp, model::SessionId};

#[tokio::test]
async fn archive_reply_keeps_its_committed_snapshot_when_a_restore_overtakes_it() {
    let registry = registry(100);
    let session = create(&registry).await;
    let service = Service::new(registry.clone());
    let mut events = session
        .try_attach(crate::session::AttachOptions {
            cursor: Some(session.meta().revision),
            replay: 0,
        })
        .await
        .expect("attach before archive");
    let mut archive = Box::pin(service.set_archived(Request::new(pb::SetArchivedRequest {
        session: session.id().to_string(),
        archived: true,
    })));
    assert!(futures::poll!(&mut archive).is_pending());
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("archive commits independently of its response waiter")
        .expect("committed archive event");
    assert!(matches!(
        event.kind,
        basr::model::EventKind::ArchiveChanged { archived: true }
    ));
    let restored = service
        .set_archived(Request::new(pb::SetArchivedRequest {
            session: session.id().to_string(),
            archived: false,
        }))
        .await
        .expect("restore")
        .into_inner()
        .session
        .expect("restored snapshot");
    let archived = archive
        .await
        .expect("earlier archive response")
        .into_inner()
        .session
        .expect("archive snapshot");
    assert!(archived.archived);
    assert!(!restored.archived);
    assert!(archived.revision < restored.revision);
    assert_eq!(archived.runtime, restored.runtime);
    registry.shutdown().await;
}

#[tokio::test]
async fn archive_rpc_filters_lists_but_preserves_identity_history_and_delete() {
    use tokio_stream::wrappers::TcpListenerStream;

    let registry = registry(100);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("address");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(Service::new(Arc::clone(&registry)).into_server())
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                let _ = stopped.await;
            }),
    );
    let mut service = basr::service::Client::connect(format!("http://{address}"))
        .await
        .expect("loopback client");
    let created = service
        .create_session(Request::new(pb::CreateSessionRequest {
            agent: "embedded".into(),
            ..Default::default()
        }))
        .await
        .expect("create over transport")
        .into_inner()
        .session
        .expect("snapshot");
    let id = created.id;
    let session = registry
        .get(id.parse().expect("session id"))
        .await
        .expect("registered");
    let archived = service
        .set_archived(Request::new(pb::SetArchivedRequest {
            session: id.clone(),
            archived: true,
        }))
        .await
        .expect("archive")
        .into_inner()
        .session
        .expect("snapshot");
    assert!(archived.archived);
    assert_eq!(archived.id, id);
    let duplicate = service
        .set_archived(Request::new(pb::SetArchivedRequest {
            session: id.clone(),
            archived: true,
        }))
        .await
        .expect("idempotent")
        .into_inner()
        .session
        .expect("snapshot");
    assert_eq!(duplicate, archived);
    for include_archived in [false, true] {
        let listed = service
            .list_sessions(Request::new(pb::ListSessionsRequest { include_archived }))
            .await
            .expect("list")
            .into_inner()
            .sessions;
        assert_eq!(listed.len(), usize::from(include_archived));
        if include_archived {
            assert_eq!(listed[0], archived);
        }
    }
    assert_eq!(registry.list().await.len(), 1);
    assert_eq!(
        service
            .get_session(Request::new(pb::GetSessionRequest {
                session: id.clone(),
            }))
            .await
            .expect("still addressable")
            .into_inner()
            .session,
        Some(archived.clone())
    );
    let history = service
        .load_history(Request::new(pb::LoadHistoryRequest {
            session: id.clone(),
            after: None,
            limit: 100,
        }))
        .await
        .expect("archived history")
        .into_inner();
    let mut attach = service
        .attach(Request::new(pb::AttachRequest {
            session: id.clone(),
            cursor: Some(0),
            replay: 100,
        }))
        .await
        .expect("archived attach")
        .into_inner();
    for event in history.events {
        assert_eq!(attach.next().await.expect("event").expect("valid"), event);
    }
    drop(attach);
    let prompt = acp::PromptRequest::new(id.clone(), vec!["hello".into()]);
    let error = service
        .prompt(Request::new(
            (&basr::model::PromptSubmission::new(prompt.clone())).into(),
        ))
        .await
        .expect("explicit rejection")
        .into_inner();
    assert!(
        matches!(error.outcome, Some(pb::prompt_submission_response::Outcome::Rejected(rejection))
        if rejection.kind == pb::PromptRejectionKind::Archived as i32)
    );
    assert_eq!(session.meta().revision, Cursor::from_raw(archived.revision));
    service
        .cancel(Request::new(
            (&acp::CancelNotification::new(id.clone())).into(),
        ))
        .await
        .expect("cancel allowed");
    let restored = service
        .set_archived(Request::new(pb::SetArchivedRequest {
            session: id.clone(),
            archived: false,
        }))
        .await
        .expect("restore")
        .into_inner()
        .session
        .expect("snapshot");
    assert_eq!(restored.id, id);
    assert!(!restored.archived);
    service
        .prompt(Request::new(
            (&basr::model::PromptSubmission::new(prompt.clone())).into(),
        ))
        .await
        .expect("recover prompt");
    service
        .set_archived(Request::new(pb::SetArchivedRequest {
            session: id.clone(),
            archived: true,
        }))
        .await
        .expect("archive running");
    service
        .delete_session(Request::new(pb::DeleteSessionRequest {
            session: id.clone(),
        }))
        .await
        .expect("true delete");
    assert!(registry.list().await.is_empty());
    assert!(registry.get(session.id()).await.is_err());
    assert_eq!(session.meta().status, basr::model::SessionStatus::Stopped);
    assert_eq!(
        service
            .get_session(Request::new(pb::GetSessionRequest {
                session: id.clone(),
            }))
            .await
            .expect_err("deleted")
            .code(),
        Code::NotFound
    );
    assert_eq!(
        service
            .set_archived(Request::new(pb::SetArchivedRequest {
                session: id,
                archived: false,
            }))
            .await
            .expect_err("cannot restore deleted")
            .code(),
        Code::NotFound
    );
    registry.shutdown().await;
    drop(service);
    stop.send(()).expect("shutdown server");
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("bounded server shutdown")
        .expect("server task")
        .expect("server result");
}

#[tokio::test]
async fn archive_rpc_uses_existing_session_error_statuses() {
    let registry = registry(100);
    let service = Service::new(Arc::clone(&registry));
    for (session, code) in [
        ("invalid-id".to_owned(), Code::InvalidArgument),
        (SessionId::new().to_string(), Code::NotFound),
    ] {
        for archived in [true, false] {
            assert_eq!(
                service
                    .set_archived(Request::new(pb::SetArchivedRequest {
                        session: session.clone(),
                        archived,
                    }))
                    .await
                    .expect_err("invalid target")
                    .code(),
                code
            );
        }
    }
    registry.shutdown().await;
}
