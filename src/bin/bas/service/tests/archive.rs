use super::*;

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
        bas::model::EventKind::StatusChanged {
            status: bas::model::SessionStatus::Stopped
        }
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
