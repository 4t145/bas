use super::*;
use bas::config::agent::Command;

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
        assert!(error.contains("本机 bas"));
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
    assert!(error.contains("启动本机 bas"));
}
