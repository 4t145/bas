//! Read-only host-side connection checks. No registry session or prompt is created.
use std::{path::PathBuf, time::Duration};

use agent_client_protocol::{
    Agent, Client, ConnectionTo, on_receive_request,
    schema::{ProtocolVersion, v1},
};

use crate::agent::AgentSpec;

/// Resolve exactly as the daemon will launch, never using a shell or GUI PATH.
/// Relative paths (including relative PATH entries) use the daemon's cwd.
pub fn resolve_program(spec: &AgentSpec) -> Option<PathBuf> {
    let command = spec.command()?;
    let path = spec
        .env
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"));
    which::which_in(&command.program, path, std::env::current_dir().ok()?).ok()
}

/// A protocol connection can succeed before the agent is ready to create sessions.
pub enum CheckResult {
    Connected(v1::InitializeResponse),
    AuthenticationRequired(v1::InitializeResponse, v1::Error),
}

/// Runs initialize and newSession with no filesystem or host terminal access.
/// Dropping the SDK transport kills/reaps its owned process group, including on
/// timeout, request cancellation and daemon shutdown. Errors are deliberately
/// classified here rather than returning agent-controlled stderr or environment.
///
/// # Errors
/// Reports unavailable executables, temporary directory failure, rejected ACP
/// initialization/session creation or expiry of the daemon's startup deadline.
pub async fn check_connection(
    spec: &AgentSpec,
    timeout: Duration,
) -> Result<CheckResult, &'static str> {
    let transport = super::convert::agent(spec).map_err(|_| "Executable could not be resolved")?;
    let directory = tempfile::tempdir().map_err(|_| "Could not create isolated probe directory")?;
    let cwd = directory.path().to_owned();
    let (ready, mut received) = tokio::sync::oneshot::channel();
    let connection = Client
        .builder()
        .name("basrd-check")
        .on_receive_request(
            async move |_request: v1::CreateTerminalRequest, _responder, _cx| {
                Err::<(), _>(agent_client_protocol::Error::into_internal_error(
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Connection checks deny terminal access",
                    ),
                ))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::RequestPermissionRequest, responder, _cx| {
                responder.respond(v1::RequestPermissionResponse::new(
                    v1::RequestPermissionOutcome::Cancelled,
                ))?;
                Ok(())
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::ReadTextFileRequest, _responder, _cx| {
                Err::<(), _>(agent_client_protocol::Error::into_internal_error(
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Connection checks deny file access",
                    ),
                ))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::WriteTextFileRequest, _responder, _cx| {
                Err::<(), _>(agent_client_protocol::Error::into_internal_error(
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Connection checks deny file access",
                    ),
                ))
            },
            on_receive_request!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            let initialized = cx
                .send_request(
                    v1::InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                        v1::ClientCapabilities::new()
                            .auth(v1::AuthCapabilities::new().terminal(true)),
                    ),
                )
                .block_task()
                .await?;
            let result = match cx
                .send_request(v1::NewSessionRequest::new(cwd))
                .block_task()
                .await
            {
                Ok(_) => CheckResult::Connected(initialized),
                Err(error) if error.code == agent_client_protocol::ErrorCode::AuthRequired => {
                    CheckResult::AuthenticationRequired(initialized, error)
                }
                Err(_) => return Ok(()),
            };
            let _ = ready.send(result);
            std::future::pending::<()>().await;
            Ok(())
        });
    // Keep transport in this future, not a detached task. Once the handshake is
    // observed, cancel transport immediately instead of waiting on a peer to exit.
    tokio::pin!(connection);
    tokio::select! {
        biased;
        result = &mut received => result.map_err(|_| "ACP initialization or newSession failed; check the agent's login"),
        _ = &mut connection => Err("ACP startup or handshake failed; verify the command and agent login"),
        () = tokio::time::sleep(timeout) => Err("ACP connection check timed out"),
    }
}
