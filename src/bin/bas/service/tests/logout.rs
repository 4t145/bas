use crate::{
    agent::{AgentSpec, Catalog, Command, LocalHost},
    service::Service,
    session::{CreateRequest, Limits, Registry},
};
use bas::{
    model::Owner,
    service::{Client, pb, pb::bas_server::Bas},
};
use std::{sync::Arc, time::Duration};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::task::AbortOnDropHandle;

struct Fixture {
    registry: Arc<Registry>,
    client: Client,
    spec: AgentSpec,
    directory: tempfile::TempDir,
    _server: AbortOnDropHandle<Result<(), tonic::transport::Error>>,
}

impl Fixture {
    async fn new(mode: &str) -> Self {
        let directory = tempfile::tempdir().expect("directory");
        let mut spec = AgentSpec::acp(
            "fixture",
            Command::new(
                "sh",
                [
                    "-c".to_owned(),
                    include_str!("../../agent/acp/assets/logout.sh").to_owned(),
                ],
            ),
        );
        spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
        spec.env.insert(
            "BAS_FIXTURE_REQUESTS".into(),
            directory
                .path()
                .join("requests")
                .to_str()
                .expect("UTF-8")
                .into(),
        );
        let registry = Arc::new(Registry::new(
            Arc::new(Catalog::from_specs([spec.clone(), AgentSpec::embedded()]).expect("catalog")),
            Arc::new(LocalHost::default()),
            Limits {
                startup_timeout: Duration::from_millis(500),
                ..Limits::default()
            },
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let server = AbortOnDropHandle::new(tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(Service::new(registry.clone()).into_server())
                .serve_with_incoming(TcpListenerStream::new(listener)),
        ));
        let client = Client::connect(format!("http://{address}"))
            .await
            .expect("client");
        Self {
            registry,
            client,
            spec,
            directory,
            _server: server,
        }
    }

    fn request(&self) -> pb::LogoutAgentRequest {
        pb::LogoutAgentRequest {
            agent: self.spec.id.to_string(),
            expected_spec_json: serde_json::to_vec(&self.spec).expect("spec"),
        }
    }

    fn methods(&self) -> Vec<String> {
        std::fs::read_to_string(self.directory.path().join("requests"))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("request")["method"]
                    .as_str()
                    .expect("method")
                    .to_owned()
            })
            .collect()
    }
}

#[tokio::test]
async fn logout_negotiates_capability_once_without_creating_or_deleting_sessions() {
    let mut fixture = Fixture::new("supported").await;
    fixture
        .registry
        .create(CreateRequest {
            agent: "embedded".into(),
            owner: Owner::current(),
            cwd: None,
            title: Some("keep".into()),
        })
        .await
        .expect("local session");
    let before = fixture.registry.list().await;
    fixture
        .client
        .logout_agent(fixture.request())
        .await
        .expect("logout");
    assert_eq!(fixture.methods(), ["initialize", "logout"]);
    assert_eq!(fixture.registry.list().await, before);
    fixture.registry.shutdown().await;
}

#[tokio::test]
async fn unsupported_logout_is_not_sent_and_native_rejection_keeps_details() {
    for mode in ["unsupported", "reject"] {
        let mut fixture = Fixture::new(mode).await;
        let error = fixture
            .client
            .logout_agent(fixture.request())
            .await
            .expect_err("failure");
        if mode == "unsupported" {
            assert_eq!(fixture.methods(), ["initialize"]);
            assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        } else {
            assert_eq!(fixture.methods(), ["initialize", "logout"]);
            assert_eq!(error.code(), tonic::Code::InvalidArgument);
            let error: bas::acp::Error =
                serde_json::from_slice(error.details()).expect("native error");
            assert_eq!(error.message, "logout rejected");
            assert_eq!(error.data.expect("data")["reason"], "fixture");
        }
    }
}

#[tokio::test]
async fn stale_configuration_unknown_peer_and_remote_peer_cannot_log_out() {
    let mut fixture = Fixture::new("supported").await;
    let mut request = fixture.request();
    let mut changed = fixture.spec.clone();
    changed.env.insert("CHANGED".into(), "true".into());
    request.expected_spec_json = serde_json::to_vec(&changed).expect("spec");
    assert_eq!(
        fixture
            .client
            .logout_agent(request)
            .await
            .expect_err("stale")
            .code(),
        tonic::Code::FailedPrecondition
    );
    let service = Service::new(fixture.registry.clone());
    assert_eq!(
        service
            .logout_agent(tonic::Request::new(fixture.request()))
            .await
            .expect_err("unknown peer")
            .code(),
        tonic::Code::PermissionDenied
    );
    let mut remote = tonic::Request::new(fixture.request());
    remote
        .extensions_mut()
        .insert(tonic::transport::server::TcpConnectInfo {
            local_addr: Some("127.0.0.1:7337".parse().expect("local")),
            remote_addr: Some("192.0.2.10:54321".parse().expect("remote")),
        });
    assert_eq!(
        service
            .logout_agent(remote)
            .await
            .expect_err("remote")
            .code(),
        tonic::Code::PermissionDenied
    );
    assert!(fixture.methods().is_empty());
}

#[tokio::test]
async fn logout_timeout_has_unknown_outcome_and_does_not_retry() {
    let mut fixture = Fixture::new("hang").await;
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        fixture.client.logout_agent(fixture.request()),
    )
    .await
    .expect("bounded RPC")
    .expect_err("timeout");
    assert_eq!(error.code(), tonic::Code::DeadlineExceeded);
    assert_eq!(fixture.methods(), ["initialize", "logout"]);
}
