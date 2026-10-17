//! Agent-to-client requests and agent-owned terminal updates for v2 demos.

use agent_client_protocol::schema::v2 as acp;
use agent_client_protocol::{Client, Error, V2ConnectionTo};
use base64::Engine as _;

use super::Demo;

/// Outcome of the scripted permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PermissionDemo {
    /// The client selected an allow option.
    Allowed,
    /// The client selected a reject option.
    Rejected,
    /// The client cancelled the prompt.
    Cancelled,
}

/// Request permission for a scripted command.
pub(super) async fn request_permission(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<PermissionDemo, Error> {
    let tool_call = acp::ToolCallUpdate::new(demo.next_tool_id())
        .title("Run the scripted demo command")
        .kind(acp::ToolKind::Execute)
        .status(acp::ToolCallStatus::Pending)
        .raw_input(serde_json::json!({"command": "echo permission demo"}));
    let request = acp::RequestPermissionRequest::new(
        session_id.clone(),
        "Allow the scripted command?",
        vec![
            acp::PermissionOption::new(
                "allow-once",
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                "allow-always",
                "Allow always",
                acp::PermissionOptionKind::AllowAlways,
            ),
            acp::PermissionOption::new(
                "reject-once",
                "Reject once",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ],
    )
    .description("The test agent will not actually run anything")
    .subject(acp::RequestPermissionSubject::ToolCall(Box::new(
        acp::ToolCallPermissionSubject::new(tool_call),
    )));
    let response = connection.send_request(request).block_task().await?;
    Ok(match response.outcome {
        acp::RequestPermissionOutcome::Selected(selected) => {
            if selected.option_id.0.starts_with("allow") {
                PermissionDemo::Allowed
            } else {
                PermissionDemo::Rejected
            }
        }
        acp::RequestPermissionOutcome::Cancelled => PermissionDemo::Cancelled,
        _ => PermissionDemo::Rejected,
    })
}

/// Send a form elicitation and return the client's action.
pub(super) async fn elicit_form(
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<acp::ElicitationAction, Error> {
    let schema = acp::ElicitationSchema::new()
        .title("Test agent form")
        .description("Answers are only used to demonstrate ACP elicitation")
        .property(
            "project",
            acp::StringPropertySchema::new()
                .title("Project")
                .description("Project name to echo back")
                .min_length(1),
            true,
        )
        .property(
            "remember",
            acp::BooleanPropertySchema::new()
                .title("Remember")
                .description("Pretend to remember this answer")
                .default_value(false),
            false,
        );
    let request = acp::CreateElicitationRequest::new(
        acp::ElicitationFormMode::new(
            acp::ElicitationSessionScope::new(session_id.clone()),
            schema,
        ),
        "Choose a project for the scripted demo",
    );
    let response = connection.send_request(request).block_task().await?;
    Ok(response.action)
}

/// Send a URL elicitation, then announce completion.
pub(super) async fn elicit_url(
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<acp::ElicitationAction, Error> {
    let elicitation_id = acp::ElicitationId::new("demo-elicitation-1");
    let request = acp::CreateElicitationRequest::new(
        acp::ElicitationUrlMode::new(
            acp::ElicitationSessionScope::new(session_id.clone()),
            elicitation_id.clone(),
            "https://example.com/acp-test-agent/authorize",
        ),
        "Open the demo URL to authorize the scripted action",
    );
    let response = connection.send_request(request).block_task().await?;
    connection.send_notification(acp::CompleteElicitationNotification::new(elicitation_id))?;
    Ok(response.action)
}

/// Simulate an agent-owned terminal, the v2 replacement for `terminal/*`
/// callbacks.
pub(super) fn exercise_terminal(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
    cwd: &std::path::Path,
) -> Result<(acp::TerminalId, String), Error> {
    let terminal_id = demo.next_terminal_id();
    let command = "echo hello from the ACP test agent";
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::TerminalUpdate(
            acp::TerminalUpdate::new(terminal_id.clone())
                .command(command)
                .cwd(cwd.to_path_buf()),
        ),
    )?;
    let output = "hello from the ACP test agent\n";
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::TerminalOutputChunk(acp::TerminalOutputChunk::new(
            terminal_id.clone(),
            base64::engine::general_purpose::STANDARD.encode(output),
        )),
    )?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::TerminalUpdate(
            acp::TerminalUpdate::new(terminal_id.clone())
                .exit_status(acp::TerminalExitStatus::new().exit_code(0)),
        ),
    )?;
    Ok((terminal_id, output.to_string()))
}
