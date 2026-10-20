use super::super::check::tests::ProbeService;
use super::*;

#[tokio::test]
async fn private_ipc_probe_requires_a_bas_rpc_and_has_no_fallback() {
    let directory = tempfile::tempdir().expect("private fixture");
    #[cfg(unix)]
    let candidate = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private permissions");
        Target::Unix(
            directory
                .path()
                .canonicalize()
                .expect("canonical directory")
                .join("probe.sock"),
        )
    };
    #[cfg(windows)]
    let candidate = Target::Pipe(format!("bas-probe-{}", uuid::Uuid::new_v4()));
    assert!(probe(candidate.clone()).await.is_err(), "absent endpoint");
    let listener = bas::service::transport::Listener::bind(&candidate)
        .await
        .expect("fixture listener");
    let server = AbortOnDropHandle::new(tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(ProbeService)
            .serve_with_incoming(listener.incoming())
            .await
            .expect("fixture server");
    }));
    assert_eq!(probe(candidate.clone()).await, Ok(candidate));
    drop(server);
}

#[cfg(unix)]
#[tokio::test]
async fn an_ipc_listener_without_a_bas_response_times_out() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("private fixture");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private permissions");
    let candidate = Target::Unix(
        directory
            .path()
            .canonicalize()
            .expect("canonical directory")
            .join("stall.sock"),
    );
    let listener = bas::service::transport::Listener::bind(&candidate)
        .await
        .expect("listener");
    let server = AbortOnDropHandle::new(tokio::spawn(async move {
        use futures::StreamExt;
        let incoming = listener.incoming();
        futures::pin_mut!(incoming);
        let _connection = incoming
            .next()
            .await
            .expect("connection")
            .expect("authenticated connection");
        std::future::pending::<()>().await;
    }));
    let start = std::time::Instant::now();
    assert!(probe(candidate).await.is_err());
    assert!(start.elapsed() < PROBE_TIMEOUT + Duration::from_secs(2));
    drop(server);
}
