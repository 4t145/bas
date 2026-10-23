//! Protocol-negotiation knobs used to test client fallback paths.

mod support;

use agent_client_protocol::schema::{ProtocolVersion, v2};
use agent_client_protocol::{Client, Error};

use support::agent_with_args;

fn args(protocol: &str) -> Vec<String> {
    vec!["--protocol".to_string(), protocol.to_string()]
}

#[tokio::test]
async fn forced_v1_rejects_a_v2_only_client() -> Result<(), Box<dyn std::error::Error>> {
    Client
        .v2()
        .name("routing-v2-client")
        .connect_with(agent_with_args(args("v1")), async |cx| {
            let result = cx
                .send_request(v2::InitializeRequest::new(
                    ProtocolVersion::V2,
                    v2::Implementation::new("routing-v2-client", "1.0.0"),
                ))
                .block_task()
                .await;
            assert!(result.is_err(), "the v2-only client must not accept v1");
            Ok::<(), Error>(())
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn forced_v2_serves_v2_clients() -> Result<(), Box<dyn std::error::Error>> {
    Client
        .v2()
        .name("routing-v2-client")
        .connect_with(agent_with_args(args("v2")), async |cx| {
            let response = cx
                .send_request(v2::InitializeRequest::new(
                    ProtocolVersion::V2,
                    v2::Implementation::new("routing-v2-client", "1.0.0"),
                ))
                .block_task()
                .await?;
            assert_eq!(response.protocol_version, ProtocolVersion::V2);
            Ok::<(), Error>(())
        })
        .await?;
    Ok(())
}
