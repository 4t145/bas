//! Authentication behavior for both protocol versions.

mod support;

use agent_client_protocol::ErrorCode;
use agent_client_protocol::schema::{ProtocolVersion, v1, v2};
use agent_client_protocol::{Client, Error};

use support::{agent_with_args, cwd};

fn require_auth() -> impl IntoIterator<Item = String> {
    ["--require-auth".to_string()]
}

#[tokio::test]
async fn v1_requires_authenticate_and_supports_logout() -> Result<(), Box<dyn std::error::Error>> {
    Client
        .builder()
        .name("v1-auth-client")
        .connect_with(agent_with_args(require_auth()), async |cx| {
            cx.send_request(v1::InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let error = cx
                .send_request(v1::NewSessionRequest::new(cwd()))
                .block_task()
                .await
                .expect_err("unauthenticated session creation fails");
            assert_eq!(error.code, ErrorCode::AuthRequired);

            let error = cx
                .send_request(v1::AuthenticateRequest::new("not-a-method"))
                .block_task()
                .await
                .expect_err("unknown auth method is rejected");
            assert_eq!(error.code, ErrorCode::InvalidParams);

            cx.send_request(v1::AuthenticateRequest::new("demo-login"))
                .block_task()
                .await?;
            cx.send_request(v1::NewSessionRequest::new(cwd()))
                .block_task()
                .await?;

            cx.send_request(v1::LogoutRequest::new())
                .block_task()
                .await?;
            let error = cx
                .send_request(v1::NewSessionRequest::new(cwd()))
                .block_task()
                .await
                .expect_err("logout requires authentication again");
            assert_eq!(error.code, ErrorCode::AuthRequired);
            Ok::<(), Error>(())
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn v2_requires_login_and_supports_logout() -> Result<(), Box<dyn std::error::Error>> {
    Client
        .v2()
        .name("v2-auth-client")
        .connect_with(agent_with_args(require_auth()), async |cx| {
            cx.send_request(v2::InitializeRequest::new(
                ProtocolVersion::V2,
                v2::Implementation::new("v2-auth-client", "1.0.0"),
            ))
            .block_task()
            .await?;
            let error = cx
                .send_request(v2::NewSessionRequest::new(cwd()))
                .block_task()
                .await
                .expect_err("unauthenticated session creation fails");
            assert_eq!(error.code, ErrorCode::AuthRequired);

            let error = cx
                .send_request(v2::LoginAuthRequest::new("not-a-method"))
                .block_task()
                .await
                .expect_err("unknown auth method is rejected");
            assert_eq!(error.code, ErrorCode::InvalidParams);

            cx.send_request(v2::LoginAuthRequest::new("demo-login"))
                .block_task()
                .await?;
            cx.send_request(v2::NewSessionRequest::new(cwd()))
                .block_task()
                .await?;

            cx.send_request(v2::LogoutAuthRequest::new())
                .block_task()
                .await?;
            let error = cx
                .send_request(v2::NewSessionRequest::new(cwd()))
                .block_task()
                .await
                .expect_err("logout requires authentication again");
            assert_eq!(error.code, ErrorCode::AuthRequired);
            Ok::<(), Error>(())
        })
        .await?;
    Ok(())
}
