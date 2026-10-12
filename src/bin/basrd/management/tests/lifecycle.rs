use super::*;
use std::{future::Future, task::Poll};

async fn poll_pending(future: std::pin::Pin<&mut impl Future>) {
    let mut future = future;
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

fn assert_clean(manager: &Manager) {
    assert!(!manager.store().expect("settings store").path().exists());
    assert!(
        fs::read_dir(manager.root.join("agents/fixture-agent"))
            .expect("versions directory")
            .next()
            .is_none(),
        "cancelled staging must be removed"
    );
    assert!(manager.mutation.try_lock().is_ok());
    assert!(manager.mutation_lock().is_ok(), "disk lock released");
    assert!(manager.installations.is_empty());
}

#[tokio::test]
async fn aborting_waiter_cancels_owned_download_and_releases_resources() {
    let fixture = fixture::Fixture::serve(None, true).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let (sender, _receiver) = mpsc::channel(32);
    let owner = Arc::clone(&manager);
    let waiter = tokio::spawn(async move { owner.install("fixture-agent", sender).await });
    fixture.started.notified().await;
    assert!(manager.mutation.try_lock().is_err());
    assert!(manager.mutation_lock().is_err());
    waiter.abort();
    assert!(waiter.await.expect_err("aborted waiter").is_cancelled());
    // Do not call shutdown here: cancellation must come from the waiter.
    manager.installations.close();
    tokio::time::timeout(Duration::from_secs(2), manager.installations.wait())
        .await
        .expect("owned operation observes dropped waiter");
    assert_clean(&manager);
    assert_eq!(Arc::strong_count(&manager), 1, "no task retains manager");
}

#[tokio::test]
async fn timed_out_waiter_cancels_queued_operation_without_effects() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, "http://127.0.0.1:1");
    let guard = manager.mutation.lock().await;
    let (sender, _receiver) = mpsc::channel(32);
    let mut waiter = Box::pin(manager.install("fixture-agent", sender));
    poll_pending(waiter.as_mut()).await;
    assert_eq!(manager.installations.len(), 1);
    assert!(tokio::time::timeout(Duration::ZERO, waiter).await.is_err());
    manager.installations.close();
    tokio::time::timeout(Duration::from_secs(2), manager.installations.wait())
        .await
        .expect("timeout cancels queued work while lock remains held");
    drop(guard);
    assert!(manager.installations.is_empty());
    assert!(!manager.root.join("operation.lock").exists());
}

#[tokio::test]
async fn shutdown_drains_active_and_queued_operations_and_rejects_new_installs() {
    let fixture = fixture::Fixture::serve(None, true).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let manager = fixture::manager(&directory, &fixture.url);
    let (sender, _receiver) = mpsc::channel(32);
    let owner = Arc::clone(&manager);
    let active = tokio::spawn(async move { owner.install("fixture-agent", sender).await });
    fixture.started.notified().await;
    let (sender, _queued_receiver) = mpsc::channel(32);
    let queued = manager.install("fixture-agent", sender);
    tokio::pin!(queued);
    poll_pending(queued.as_mut()).await;
    assert_eq!(manager.installations.len(), 2);
    tokio::time::timeout(Duration::from_secs(2), manager.shutdown())
        .await
        .expect("shutdown drains active and queued work");
    assert!(active.await.expect("active waiter").is_err());
    assert!(queued.await.is_err());
    assert_clean(&manager);
    let (sender, _receiver) = mpsc::channel(32);
    assert!(
        manager
            .install("fixture-agent", sender)
            .await
            .expect_err("shutdown rejects new work")
            .to_string()
            .contains("cancelled")
    );
    assert_clean(&manager);
}

#[tokio::test]
async fn cancellation_unblocks_full_progress_queue() {
    let cancellation = CancellationToken::new();
    let (sender, _receiver) = mpsc::channel(1);
    let progress = Progress {
        sender: Some(sender),
        cancellation: cancellation.clone(),
    };
    progress.send("first", "fills queue").await.expect("send");
    let blocked = progress.send("second", "waits");
    tokio::pin!(blocked);
    poll_pending(blocked.as_mut()).await;
    cancellation.cancel();
    assert!(blocked.await.is_err());
    assert!(progress.check().is_err());
}
