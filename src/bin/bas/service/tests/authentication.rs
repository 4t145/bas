use std::{sync::Arc, time::Duration};

use bas::service::{Client, pb};
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};

use super::super::Service;
use crate::{
    agent::{AgentSpec, Catalog, Command, LocalHost},
    session::{Limits, Registry},
};

fn spec(directory: &tempfile::TempDir) -> AgentSpec {
    let mut spec = AgentSpec::acp(
        "fixture",
        Command::new(
            "sh",
            [
                "-c".into(),
                include_str!("../../agent/acp/assets/fixture.sh").into(),
            ],
        ),
    );
    spec.env
        .insert("BAS_FIXTURE_MODE".into(), "agent_auth_form".into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
        directory.path().join("pid").to_string_lossy().into_owned(),
    );
    spec
}

async fn server(
    spec: AgentSpec,
) -> (
    Arc<Registry>,
    Client,
    tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
) {
    let registry = Arc::new(Registry::new(
        Arc::new(Catalog::from_specs([spec]).expect("catalog")),
        Arc::new(LocalHost::default()),
        Limits::default(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(Service::new(registry.clone()).into_server())
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let client = Client::connect(format!("http://{address}"))
        .await
        .expect("client");
    (registry, client, task)
}

fn start(spec: &AgentSpec) -> pb::AuthenticationInput {
    pb::AuthenticationInput {
        kind: Some(pb::authentication_input::Kind::Start(
            pb::AuthenticateAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: serde_json::to_vec(spec).expect("JSON"),
                method_id: "login".into(),
                expected_method_json: Vec::new(),
            },
        )),
    }
}

#[tokio::test]
async fn grpc_interactive_authentication_routes_a_form_and_final_check() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = spec(&directory);
    let (registry, mut client, server) = server(spec.clone()).await;
    let (controls, input) = tokio::sync::mpsc::channel(8);
    controls.send(start(&spec)).await.expect("start");
    let mut output = client
        .authenticate_agent_interactive(ReceiverStream::new(input))
        .await
        .expect("interactive RPC")
        .into_inner();
    let event = tokio::time::timeout(Duration::from_secs(5), output.message())
        .await
        .expect("timeout")
        .expect("RPC event")
        .expect("form");
    let Some(pb::authentication_event::Kind::Elicitation(pending)) = event.kind else {
        panic!("expected form");
    };
    controls
        .send(pb::AuthenticationInput {
            kind: Some(pb::authentication_input::Kind::ElicitationResponse(
                pb::ElicitationAnswer {
                    request_id: pending.request_id,
                    response: Some(
                        (&serde_json::json!({"action":"accept","content":{"project":"atlas"}}))
                            .into(),
                    ),
                },
            )),
        })
        .await
        .expect("answer");
    let event = tokio::time::timeout(Duration::from_secs(5), output.message())
        .await
        .expect("timeout")
        .expect("RPC event")
        .expect("check");
    let Some(pb::authentication_event::Kind::Check(check)) = event.kind else {
        panic!("expected check");
    };
    assert_eq!(check.status(), pb::AgentCheckStatus::Connected);
    assert!(
        registry.list().await.is_empty(),
        "authentication creates no Bas session"
    );
    assert!(!directory.path().join("pid.prompt").exists());
    drop(output);
    registry.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn unary_and_interactive_authentication_preserve_readiness_details() {
    for (mode, status) in [
        ("agent_auth", pb::AgentCheckStatus::Connected),
        (
            "agent_auth_still_required",
            pb::AgentCheckStatus::AuthenticationRequired,
        ),
    ] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let mut spec = spec(&directory);
        spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
        let (registry, mut client, server) = server(spec.clone()).await;
        let Some(pb::authentication_input::Kind::Start(request)) = start(&spec).kind else {
            panic!("start request");
        };
        let unary =
            tokio::time::timeout(Duration::from_secs(5), client.authenticate_agent(request))
                .await
                .expect("unary authentication deadline")
                .expect("unary authentication")
                .into_inner()
                .check
                .expect("unary check");

        let (controls, input) = tokio::sync::mpsc::channel(8);
        controls.send(start(&spec)).await.expect("start");
        let mut output = client
            .authenticate_agent_interactive(ReceiverStream::new(input))
            .await
            .expect("interactive RPC")
            .into_inner();
        let event = tokio::time::timeout(Duration::from_secs(5), output.message())
            .await
            .expect("interactive authentication deadline")
            .expect("RPC event")
            .expect("check");
        let Some(pb::authentication_event::Kind::Check(interactive)) = event.kind else {
            panic!("expected check");
        };
        assert_eq!(unary.status(), status);
        assert_eq!(interactive.status(), status);
        assert_eq!(
            unary.message,
            "Authentication flow completed; session readiness checked"
        );
        assert_eq!(
            interactive.message,
            "Agent setup completed; readiness checked"
        );
        assert!(!unary.resolved_program.is_empty());
        assert_eq!(unary.resolved_program, interactive.resolved_program);
        assert!(unary.initialization.is_some());
        assert_eq!(unary.initialization, interactive.initialization);
        assert_eq!(
            unary.authentication_error.is_some(),
            status == pb::AgentCheckStatus::AuthenticationRequired
        );
        assert_eq!(unary.authentication_error, interactive.authentication_error);
        assert!(registry.list().await.is_empty());
        assert!(!directory.path().join("pid.prompt").exists());
        drop(output);
        drop(controls);
        registry.shutdown().await;
        server.abort();
    }
}

#[tokio::test]
async fn agent_operations_reject_invalid_or_stale_expected_specs_before_launch() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let spec = spec(&directory);
    let (registry, mut client, server) = server(spec.clone()).await;
    let mut stale = spec.clone();
    stale.env.insert("CHANGED".into(), "value".into());
    for (bytes, code, message) in [
        (
            b"not-json".to_vec(),
            tonic::Code::InvalidArgument,
            "expected_spec_json must contain a saved AgentSpec",
        ),
        (
            b"null".to_vec(),
            tonic::Code::InvalidArgument,
            "expected_spec_json must contain a saved AgentSpec",
        ),
        (
            serde_json::to_vec(&stale).expect("stale spec"),
            tonic::Code::FailedPrecondition,
            "Agent configuration changed; retry",
        ),
    ] {
        let error = client
            .check_agent(pb::CheckAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: bytes.clone(),
                connect: true,
            })
            .await
            .expect_err("check rejects invalid or stale spec");
        assert_eq!(error.code(), code);
        assert_eq!(
            error.message(),
            if code == tonic::Code::FailedPrecondition {
                "Agent configuration changed; save and retry"
            } else {
                message
            }
        );
        let error = client
            .logout_agent(pb::LogoutAgentRequest {
                agent: spec.id.to_string(),
                expected_spec_json: bytes.clone(),
            })
            .await
            .expect_err("logout rejects invalid or stale spec");
        assert_eq!(error.code(), code);
        assert_eq!(error.message(), message);
        let request = pb::AuthenticateAgentRequest {
            agent: spec.id.to_string(),
            expected_spec_json: bytes,
            method_id: "login".into(),
            expected_method_json: Vec::new(),
        };
        let error = client
            .authenticate_agent(request.clone())
            .await
            .expect_err("authentication rejects invalid or stale spec");
        assert_eq!(error.code(), code);
        assert_eq!(error.message(), message);
        let error = client
            .authenticate_agent_interactive(tokio_stream::iter([pb::AuthenticationInput {
                kind: Some(pb::authentication_input::Kind::Start(request)),
            }]))
            .await
            .expect_err("interactive authentication rejects invalid or stale spec");
        assert_eq!(error.code(), code);
        assert_eq!(error.message(), message);
    }
    assert!(!directory.path().join("pid").exists());
    assert!(registry.list().await.is_empty());
    registry.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn grpc_authentication_failures_preserve_stages_and_protocol_reasons() {
    for (mode, stage, code, reason, authenticated) in [
        (
            "agent_auth_rpc_initialize_reject",
            "ACP initialization failed",
            -32603,
            "Provider initialization failed",
            false,
        ),
        (
            "agent_auth_rpc_discovery_reject",
            "Session discovery failed before authentication",
            -32602,
            "Provider project is missing",
            false,
        ),
        (
            "agent_auth_rpc_reject",
            "Agent authentication failed",
            -32603,
            "Provider rejected this authentication method",
            true,
        ),
        (
            "agent_auth_rpc_reject_details",
            "Agent authentication failed",
            -32603,
            "Could not save authentication settings",
            true,
        ),
        (
            "agent_auth_rpc_after_reject",
            "Session creation failed after authentication",
            -32603,
            "Provider cannot create a session",
            true,
        ),
    ] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let mut spec = spec(&directory);
        spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
        let (registry, mut client, server) = server(spec.clone()).await;
        let Some(pb::authentication_input::Kind::Start(request)) = start(&spec).kind else {
            panic!("start request");
        };
        let error =
            tokio::time::timeout(Duration::from_secs(5), client.authenticate_agent(request))
                .await
                .expect("unary failure deadline")
                .expect_err("Agent's failed RPC stays a failure");
        assert_authentication_failure(&error, stage, code, reason);
        assert!(directory.path().join("pid.initialize").exists());
        assert_eq!(
            directory.path().join("pid.authenticate").exists(),
            authenticated
        );

        let (controls, input) = tokio::sync::mpsc::channel(8);
        controls.send(start(&spec)).await.expect("start");
        let mut output = client
            .authenticate_agent_interactive(ReceiverStream::new(input))
            .await
            .expect("interactive RPC")
            .into_inner();
        let error = tokio::time::timeout(Duration::from_secs(5), output.message())
            .await
            .expect("interactive failure deadline")
            .expect_err("Agent's failed RPC stays a failure");
        assert_authentication_failure(&error, stage, code, reason);
        assert!(
            registry.list().await.is_empty(),
            "authentication creates no Bas sessions"
        );
        assert!(!directory.path().join("pid.prompt").exists());
        drop(output);
        drop(controls);
        registry.shutdown().await;
        server.abort();
    }
}

fn assert_authentication_failure(error: &tonic::Status, stage: &str, code: i32, reason: &str) {
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    assert!(error.message().starts_with(stage), "failure stage: {error}");
    assert!(
        error.message().contains(&format!("ACP error {code}")),
        "protocol code: {error}"
    );
    assert!(
        error.message().contains(reason),
        "provider diagnostic: {error}"
    );
    for omitted in [
        "private",
        "access credential",
        "refresh credential",
        "stderr",
        "stack",
        "API_KEY",
    ] {
        assert!(
            !error.message().contains(omitted),
            "RPC data field leaked: {omitted}"
        );
    }
}
