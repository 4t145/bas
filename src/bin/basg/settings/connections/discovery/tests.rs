use super::*;
use tonic::{Request, Response, Status, codegen::*};

#[derive(Clone)]
struct ProbeService;

impl tonic::server::NamedService for ProbeService {
    const NAME: &'static str = "bas.v1.Bas";
}

impl tonic::server::UnaryService<pb::ListAgentsRequest> for ProbeService {
    type Response = pb::ListAgentsResponse;
    type Future = BoxFuture<Response<Self::Response>, Status>;

    fn call(&mut self, _: Request<pb::ListAgentsRequest>) -> Self::Future {
        Box::pin(async { Ok(Response::new(pb::ListAgentsResponse::default())) })
    }
}

impl Service<http::Request<tonic::body::Body>> for ProbeService {
    type Response = http::Response<tonic::body::Body>;
    type Error = std::convert::Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        assert_eq!(request.uri().path(), "/bas.v1.Bas/ListAgents");
        Box::pin(async move {
            let codec = tonic_prost::ProstCodec::default();
            Ok(tonic::server::Grpc::new(codec)
                .unary(ProbeService, request)
                .await)
        })
    }
}

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
