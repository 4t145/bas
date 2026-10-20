use super::*;

#[tokio::test]
async fn unreachable_tcp_daemon_reports_connection_failure_not_local_only_capability() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("unused port");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let error = request(
        address.to_string().parse().expect("TCP target"),
        AgentSpec::embedded(),
        false,
        "/not/a/client/settings/file".into(),
    )
    .await
    .expect_err("connection failure");
    assert_ne!(error, crate::i18n::tr!("agent-check-local"));
}
