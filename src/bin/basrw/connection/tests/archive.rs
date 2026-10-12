use super::*;

#[tokio::test]
async fn catalog_requests_and_keeps_archived_metadata() {
    let fixture = fixture(Scenario::Archived).await;
    let (updates, mut receiver) = mpsc::channel(1);
    let task = tokio::spawn(catalog_loop(
        fixture.client.clone(),
        updates,
        Arc::new(Notify::new()),
    ));
    let result = tokio::time::timeout(Duration::from_secs(5), receiver.recv()).await;
    task.abort();
    let Some(Update::Catalog(Ok(catalog))) = result.expect("catalog timeout") else {
        panic!("expected full catalog");
    };
    assert_eq!(catalog.sessions.len(), 1);
    assert_eq!(catalog.sessions[0].id, fixture.session);
    assert!(catalog.sessions[0].archived);
}

#[tokio::test]
async fn old_daemon_archive_is_actionable_and_never_deletes() {
    let fixture = fixture(Scenario::Reconnect).await;
    let update = execute(
        fixture.client.clone(),
        Command::Archive {
            session: fixture.session,
            archived: true,
        },
    )
    .await;
    let Update::Archived {
        result: Err(error), ..
    } = update
    else {
        panic!("old daemon must report unsupported archive");
    };
    assert!(error.contains("升级并重启 basrd"));
    assert!(error.contains("清除内存模式的会话"));
    assert!(error.contains("归档"));
}
