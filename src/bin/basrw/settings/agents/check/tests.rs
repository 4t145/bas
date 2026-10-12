use super::*;
use basr::config::agent::Command;

#[test]
fn availability_probe_login_and_previous_results_have_separate_labels() {
    assert_eq!(Status::Installed("/tool".into()).label(), "已安装 · 待检测");
    assert_eq!(Status::Installed("/tool".into()).check_label(), "检测");
    assert_eq!(Status::Connecting.label(), "正在检测…");
    assert_eq!(Status::Connected("/tool".into()).label(), "检测通过");
    assert_eq!(Status::Connected("/tool".into()).check_label(), "重新检测");
    let login = Status::AuthenticationRequired {
        program: "/tool".into(),
        methods: vec![],
    };
    assert_eq!(login.label(), "需要认证");
    assert_eq!(login.check_label(), "重新检测");
    assert!(Status::Connecting.busy());
    assert!(Status::Authenticating.busy());
    assert!(!login.busy());
}

#[tokio::test]
async fn local_configuration_is_never_sent_to_a_remote_daemon() {
    let mut spec = AgentSpec::acp("custom", Command::new("/local/adapter", []));
    spec.env.insert("PRIVATE".into(), "fixture-secret".into());
    for connect in [false, true] {
        let error = request(
            "192.0.2.1:7337".parse().expect("test address"),
            spec.clone(),
            connect,
            "/fixture/settings.json".into(),
        )
        .await
        .expect_err("remote endpoint must be rejected without a network request");
        assert!(error.contains("本机 basrd"));
        assert!(!error.contains("fixture-secret"));
    }
}

#[tokio::test]
async fn unreachable_local_daemon_is_not_reported_as_installed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local socket");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let error = request(
        address,
        AgentSpec::embedded(),
        false,
        "/fixture/settings.json".into(),
    )
    .await
    .expect_err("not connected");
    assert!(error.contains("启动本机 basrd"));
}

#[test]
fn session_specific_auth_methods_override_initialization_without_mutating_it() {
    use basr::acp;
    let initial: acp::InitializeResponse = serde_json::from_value(serde_json::json!({
        "protocolVersion":1, "agentCapabilities":{},
        "authMethods":[{"id":"initial","name":"Initial","type":"terminal","args":["old"]}]
    }))
    .expect("initialize");
    let methods = authentication_methods(
        serde_json::to_value(initial.clone()).expect("raw initial"),
        None,
    )
    .expect("fallback");
    assert_eq!(methods[0].id, "initial");
    let error = acp::Error::auth_required().data(serde_json::json!({
        "authMethods":[{"id":"specific","name":"Specific","type":"terminal","args":["new"]}]
    }));
    let methods = authentication_methods(
        serde_json::to_value(initial.clone()).expect("raw initial"),
        Some((&error).into()),
    )
    .expect("specific");
    assert_eq!(methods[0].id, "specific");
    assert_eq!(initial.auth_methods[0].id().to_string(), "initial");
    let malformed = acp::Error::auth_required().data(serde_json::json!({"authMethods":false}));
    assert!(
        authentication_methods(
            serde_json::to_value(initial).expect("raw initial"),
            Some((&malformed).into())
        )
        .is_err()
    );
}
