use super::*;
use tonic::{Request, Response, Status, codegen::*};

#[cfg(feature = "gui-tests")]
mod gui;

#[derive(Clone)]
pub(in crate::settings::connections) struct ProbeService;

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

struct Server {
    target: Target,
    task: AbortOnDropHandle<()>,
}

impl Server {
    async fn start(tls: Option<tonic::transport::ServerTlsConfig>) -> Self {
        let listener = bas::service::transport::Listener::bind(
            &"tcp://127.0.0.1:0".parse().expect("test endpoint"),
        )
        .await
        .expect("test listener");
        let target = listener.target().clone();
        let mut builder = tonic::transport::Server::builder();
        if let Some(tls) = tls {
            builder = builder.tls_config(tls).expect("TLS fixture");
        }
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            builder
                .add_service(ProbeService)
                .serve_with_incoming(listener.incoming())
                .await
                .expect("test service");
        }));
        Self { target, task }
    }
}

#[tokio::test]
async fn tcp_checks_require_rpc_success_and_bound_stalled_peers() {
    let server = Server::start(None).await;
    let options: ConnectionOptions = server.target.clone().into();
    probe(options.clone(), CHECK_TIMEOUT)
        .await
        .expect("real RPC");
    server.task.abort();
    let _ = server.task.await;
    let error = probe(options.clone(), CHECK_TIMEOUT)
        .await
        .expect_err("closed listener");
    assert_eq!(
        error.message(&options.target),
        crate::i18n::tr!("connections-check-refused")
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stall listener");
    let target = Target::from(listener.local_addr().expect("address"));
    let task = AbortOnDropHandle::new(tokio::spawn(async move {
        let _connection = listener.accept().await.expect("connection");
        std::future::pending::<()>().await;
    }));
    assert!(matches!(
        probe(target.into(), Duration::from_millis(50)).await,
        Err(Error::Timeout)
    ));
    drop(task);
}

#[tokio::test]
async fn mtls_checks_use_the_profiles_certificate_files_and_expected_name() {
    use bas::service::tls::{ClientTls, ServerTls};
    use rcgen::{BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair};
    let directory = tempfile::tempdir().expect("temporary certificate directory");
    let key = KeyPair::generate().expect("CA key");
    let mut parameters = CertificateParams::default();
    parameters.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = parameters.self_signed(&key).expect("CA");
    let ca_path = directory.path().join("ca.pem");
    std::fs::write(&ca_path, ca.pem()).expect("CA file");
    for (name, usage) in [
        ("server", ExtendedKeyUsagePurpose::ServerAuth),
        ("client", ExtendedKeyUsagePurpose::ClientAuth),
    ] {
        let leaf_key = KeyPair::generate().expect("leaf key");
        let mut parameters = CertificateParams::new(vec!["localhost".into()]).expect("SAN");
        parameters.extended_key_usages = vec![usage];
        let leaf = parameters
            .signed_by(&leaf_key, &ca, &key)
            .expect("leaf certificate");
        std::fs::write(directory.path().join(format!("{name}.pem")), leaf.pem())
            .expect("certificate");
        std::fs::write(
            directory.path().join(format!("{name}.key")),
            leaf_key.serialize_pem(),
        )
        .expect("temporary key");
    }
    let tls = ServerTls {
        certificate: directory.path().join("server.pem"),
        private_key: directory.path().join("server.key"),
        client_ca: ca_path.clone(),
    };
    let server = Server::start(Some(tls.load().expect("server TLS"))).await;
    let Target::Tcp(address) = server.target.clone() else {
        unreachable!()
    };
    let mut options = ConnectionOptions {
        target: Target::Mtls(address),
        tls: Some(ClientTls {
            ca: ca_path,
            certificate: directory.path().join("client.pem"),
            private_key: directory.path().join("client.key"),
            server_name: Some("localhost".into()),
        }),
    };
    probe(options.clone(), CHECK_TIMEOUT)
        .await
        .expect("mutual TLS RPC");
    options.tls.as_mut().expect("TLS").server_name = Some("wrong.invalid".into());
    assert!(probe(options.clone(), CHECK_TIMEOUT).await.is_err());
    options.tls.as_mut().expect("TLS").private_key = directory.path().join("missing.key");
    let error = probe(options, CHECK_TIMEOUT)
        .await
        .expect_err("missing credential file");
    assert!(matches!(
        error,
        Error::Connect(bas::service::tls::Error::Read { .. })
    ));
}

#[test]
fn rpc_failures_do_not_display_debug_stacks_or_remote_payloads() {
    for code in [
        tonic::Code::Internal,
        tonic::Code::Unavailable,
        tonic::Code::PermissionDenied,
    ] {
        let error = Error::Rpc(Status::new(code, "fixture-private-response"));
        let message = error.message(&"tcp://test-host".parse().expect("target"));
        assert!(!message.contains("fixture-private-response"));
        assert!(!message.contains("source:"));
    }
}
