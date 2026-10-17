//! Agent-to-client requests exercised by the v1 prompt demos.

use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::{Client, ConnectionTo};

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
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<PermissionDemo, acp::Error> {
    let tool_call = acp::ToolCallUpdate::new(
        demo.next_tool_id(),
        acp::ToolCallUpdateFields::new()
            .title("Run the scripted demo command")
            .kind(acp::ToolKind::Execute)
            .status(acp::ToolCallStatus::Pending)
            .raw_input(serde_json::json!({"command": "echo permission demo"})),
    );
    let request = acp::RequestPermissionRequest::new(
        session_id.clone(),
        tool_call,
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
    );
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
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<acp::ElicitationAction, acp::Error> {
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
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(acp::ElicitationAction, acp::ElicitationId), acp::Error> {
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
    connection.send_notification(acp::CompleteElicitationNotification::new(
        elicitation_id.clone(),
    ))?;
    Ok((response.action, elicitation_id))
}

/// Exercise the v1 file-system callbacks and summarize what happened.
fn capabilities_allow_write(demo: &Demo) -> bool {
    demo.client_capabilities()
        .is_none_or(|capabilities| capabilities.fs.write_text_file)
}

pub(super) async fn exercise_filesystem(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    cwd: &std::path::Path,
) -> Result<String, acp::Error> {
    if let Some(capabilities) = demo.client_capabilities()
        && !capabilities.fs.read_text_file
    {
        return Ok("client did not advertise fs.readTextFile".to_string());
    }
    let path = cwd.join("acp-test-agent-note.txt");
    let write_result = if capabilities_allow_write(demo) {
        match connection
            .send_request(acp::WriteTextFileRequest::new(
                session_id.clone(),
                path.clone(),
                "written by acp-test-agent for the fs demo\n",
            ))
            .block_task()
            .await
        {
            Ok(_) => "wrote 1 line".to_string(),
            Err(error) => format!("write failed: {error}"),
        }
    } else {
        "client did not advertise fs.writeTextFile".to_string()
    };
    let read = connection
        .send_request(
            acp::ReadTextFileRequest::new(session_id.clone(), path)
                .line(1)
                .limit(2),
        )
        .block_task()
        .await;
    match read {
        Ok(response) => Ok(format!(
            "{write_result}; read back {} bytes: {}",
            response.content.len(),
            response.content.trim()
        )),
        Err(error) => Ok(format!("{write_result}; read failed: {error}")),
    }
}

/// Exercise the v1 client-managed terminal callbacks.
pub(super) async fn exercise_terminal(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<TerminalDemo, acp::Error> {
    if let Some(capabilities) = demo.client_capabilities()
        && !capabilities.terminal
    {
        return Ok(TerminalDemo::Unsupported);
    }
    let created = connection
        .send_request(
            acp::CreateTerminalRequest::new(session_id.clone(), "echo")
                .args(vec!["hello from the ACP test agent".to_string()]),
        )
        .block_task()
        .await?;
    let terminal_id = created.terminal_id;
    let output = connection
        .send_request(acp::TerminalOutputRequest::new(
            session_id.clone(),
            terminal_id.clone(),
        ))
        .block_task()
        .await
        .map(|response| response.output)
        .unwrap_or_else(|error| format!("output failed: {error}"));
    let exit = connection
        .send_request(acp::WaitForTerminalExitRequest::new(
            session_id.clone(),
            terminal_id.clone(),
        ))
        .block_task()
        .await
        .map(|response| response.exit_status.exit_code)
        .unwrap_or_default();
    // Embed the terminal before releasing it: released terminals must no
    // longer be referenced from tool-call content.
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new(demo.next_tool_id(), "Run a client-owned terminal")
                .kind(acp::ToolKind::Execute)
                .status(acp::ToolCallStatus::Completed)
                .content(vec![acp::ToolCallContent::Terminal(acp::Terminal::new(
                    terminal_id.clone(),
                ))]),
        ),
    )?;
    let _ = connection
        .send_request(acp::ReleaseTerminalRequest::new(
            session_id.clone(),
            terminal_id.clone(),
        ))
        .block_task()
        .await;

    // A second terminal is killed before release to cover the kill callback.
    let killed = connection
        .send_request(
            acp::CreateTerminalRequest::new(session_id.clone(), "sleep").args(vec!["60".into()]),
        )
        .block_task()
        .await
        .ok();
    if let Some(killed) = killed {
        let _ = connection
            .send_request(acp::KillTerminalRequest::new(
                session_id.clone(),
                killed.terminal_id.clone(),
            ))
            .block_task()
            .await;
        let _ = connection
            .send_request(acp::ReleaseTerminalRequest::new(
                session_id.clone(),
                killed.terminal_id,
            ))
            .block_task()
            .await;
    }

    Ok(TerminalDemo::Output {
        terminal_id,
        output,
        exit_code: exit,
    })
}

/// Result of the terminal demo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TerminalDemo {
    /// Client did not advertise terminal support.
    Unsupported,
    /// The demo ran and produced output.
    Output {
        /// Terminal id, for embedding in a tool-call update.
        terminal_id: acp::TerminalId,
        /// Captured output.
        output: String,
        /// Exit code reported by the client.
        exit_code: Option<u32>,
    },
}
