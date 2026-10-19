use crate::{
    agent::{AgentSpec, Catalog, Command, LocalHost},
    service::Service,
    session::{CreateRequest, Limits, Registry},
};
use bas::{
    model::Owner,
    service::{Client, pb},
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
