use super::*;

#[tokio::test]
async fn running_embedded_turn_survives_archive_and_restore_keeps_history() {
    let registry = registry();
    let session = registry.create(request()).await.expect("create embedded");
    let mut live = session.attach(options(0)).await.expect("attach");
    let turn = session
        .prompt(vec!["first conversation".into()])
        .await
        .expect("prompt");
    let before = session.meta();
    let history = session.page(None, 100).expect("history").events;

    let archived = session.set_archived(true).await;
    assert!(archived.archived);
    assert!(matches!(
        archived.status,
        SessionStatus::Working | SessionStatus::Idle
    ));
    if archived.status == SessionStatus::Working {
        assert_eq!(archived.active_turn, Some(turn));
    }
    assert!(archived.revision > before.revision);
    assert!(session.set_archived(true).await.archived);
    assert!(matches!(
        session.prompt(vec!["blocked".into()]).await,
        Err(SubmissionError::Rejected(PromptRejection {
            kind: PromptRejectionKind::Archived,
            ..
        }))
    ));
    assert!(session.meta().archived);
    assert_eq!(registry.list().await.len(), 1);
    assert_eq!(
        finish(&mut live).await.expect("not cancelled"),
        StopReason::EndTurn
    );
    assert!(session.meta().archived);
    assert_eq!(session.meta().status, SessionStatus::Idle);
    let page = session.page(None, 100).expect("retained history");
    assert_eq!(&page.events[..history.len()], history.as_slice());
    assert!(page.events.iter().any(|event| matches!(
        &event.kind,
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(_))
    )));
    let mut replay = session
        .attach(options(100))
        .await
        .expect("archived attachment");
    for event in &page.events {
        assert_eq!(&replay.recv().await.expect("replay"), event);
    }

    let restored = session.set_archived(false).await;
    assert_eq!(restored.id, before.id);
    assert!(!restored.archived);
    assert_eq!(restored.status, SessionStatus::Idle);
    assert_eq!(session.set_archived(false).await, restored);
    session
        .prompt(vec!["continue".into()])
        .await
        .expect("same backend accepts next turn");
    assert_eq!(
        finish(&mut live).await.expect("continued"),
        StopReason::EndTurn
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn archival_is_metadata_for_starting_stopped_and_failed_sessions() {
    for status in [
        SessionStatus::Starting,
        SessionStatus::Stopped,
        SessionStatus::Failed,
    ] {
        let session = Session::new(
            SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current()),
            Limits::default(),
            None,
        );
        match status {
            SessionStatus::Stopped => session.shutdown().await,
            SessionStatus::Failed => session.fail("fixture failure".into()).await,
            _ => {}
        }
        let before = session.meta();
        let archived = session.set_archived(true).await;
        assert_eq!(archived.status, status);
        assert_eq!(archived.active_turn, before.active_turn);
        assert_eq!(archived.revision, before.revision.next());
        assert_eq!(session.set_archived(true).await, archived);
        assert!(matches!(
            session.prompt(vec!["blocked".into()]).await,
            Err(SubmissionError::Rejected(PromptRejection {
                kind: PromptRejectionKind::Archived,
                ..
            }))
        ));
        let restored = session.set_archived(false).await;
        assert_eq!(restored.status, status);
        assert_eq!(restored.revision, archived.revision.next());
        assert_eq!(session.set_archived(false).await, restored);
        assert!(matches!(
            session.prompt(vec!["not revived".into()]).await,
            Err(SubmissionError::Rejected(PromptRejection {
                kind: PromptRejectionKind::Unavailable,
                ..
            }))
        ));
        let page = session.page(None, 100).expect("terminal history");
        let mut replay = session.attach(options(100)).await.expect("terminal attach");
        for event in page.events {
            assert_eq!(replay.recv().await.expect("terminal replay"), event);
        }
        if status != SessionStatus::Starting {
            assert_eq!(replay.recv().await, Err(AttachError::Closed));
        }
    }
}
