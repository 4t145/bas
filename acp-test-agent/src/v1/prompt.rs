//! v1 prompt handling: accept a prompt, run one scripted demo, respond.

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::{Client, ConnectionTo, Responder};

use super::Demo;
use super::client_ops::{self, PermissionDemo, TerminalDemo};
use super::updates::{available_commands, config_options};
use crate::demo::commands::{self, Command, StopReason as DemoStopReason};
use crate::demo::{self, SAMPLE_AUDIO_BASE64, SAMPLE_IMAGE_BASE64, SAMPLE_RESOURCE};
use crate::extensions::{DemoNotice, EchoRequest};

/// Result of one prompt turn.
struct Turn {
    stop_reason: acp::StopReason,
    usage: Option<acp::Usage>,
}

impl Turn {
    fn end_turn() -> Self {
        Self {
            stop_reason: acp::StopReason::EndTurn,
            usage: None,
        }
    }

    fn with(reason: acp::StopReason) -> Self {
        Self {
            stop_reason: reason,
            usage: None,
        }
    }
}

/// Handle `session/prompt`.
///
/// Validation happens in the dispatch loop, then the turn runs in a spawned task so
/// `session/cancel` can still be received. The response is sent when the turn ends.
pub(super) fn handle_prompt(
    demo: &Arc<Demo>,
    connection: ConnectionTo<Client>,
    request: acp::PromptRequest,
    responder: Responder<acp::PromptResponse>,
) -> Result<(), acp::Error> {
    let session_id = request.session_id.0.to_string();
    if let Err(error) = demo.store().begin_prompt(&session_id) {
        return responder.respond_with_error(acp::Error::invalid_params().data(error.to_string()));
    }
    let cancellation = responder.cancellation();
    let worker_demo = demo.clone();
    let worker_session_id = session_id.clone();
    if let Err(error) = connection.clone().spawn(async move {
        let result = run_turn(&worker_demo, &connection, &request, &cancellation).await;
        worker_demo
            .store()
            .finish_prompt(worker_session_id.as_str());
        match result {
            Ok(turn) => {
                let mut response = acp::PromptResponse::new(turn.stop_reason);
                if let Some(usage) = turn.usage {
                    response = response.usage(usage);
                }
                responder.respond(response)
            }
            Err(error) => responder.respond_with_error(error),
        }
    }) {
        demo.store().finish_prompt(session_id.as_str());
        return Err(error);
    }
    Ok(())
}

async fn run_turn(
    demo: &Arc<Demo>,
    connection: &ConnectionTo<Client>,
    request: &acp::PromptRequest,
    cancellation: &agent_client_protocol::RequestCancellation,
) -> Result<Turn, acp::Error> {
    let session_id = &request.session_id;
    let command = commands::parse(&prompt_text(&request.prompt));
    let (cwd, config) = {
        let store = demo.store();
        let session = store
            .get(session_id.0.as_ref())
            .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
        (session.cwd.clone(), session.config.clone())
    };
    let mut turn = match command {
        Command::Help => run_help(demo, connection, session_id)?,
        Command::Echo { text } => run_echo(demo, connection, session_id, &config, &text)?,
        Command::Rich => run_rich(demo, connection, session_id)?,
        Command::Thought => run_thought(demo, connection, session_id)?,
        Command::Tool => run_tool(demo, connection, session_id, &cwd)?,
        Command::Plan { millis } => {
            run_plan(demo, connection, session_id, millis, cancellation).await?
        }
        Command::ClearPlan => {
            remove_plan(demo, connection, session_id)?;
            send_text(demo, connection, session_id, "Plan cleared.")?;
            Turn::end_turn()
        }
        Command::Permission => run_permission(demo, connection, session_id).await?,
        Command::Elicitation => run_elicitation(demo, connection, session_id).await?,
        Command::ElicitationUrl => run_elicitation_url(demo, connection, session_id).await?,
        Command::FileSystem => run_filesystem(demo, connection, session_id, &cwd).await?,
        Command::Terminal => run_terminal(demo, connection, session_id).await?,
        Command::Sleep { millis } => {
            run_sleep(demo, connection, session_id, millis, cancellation).await?
        }
        Command::Config { setting } => {
            run_config(demo, connection, session_id, setting.as_deref())?
        }
        Command::Commands => run_commands(demo, connection, session_id)?,
        Command::Usage => run_usage(demo, connection, session_id)?,
        Command::Notice => run_notice(demo, connection, session_id)?,
        Command::Compaction => run_compaction(demo, connection, session_id)?,
        Command::Info => run_info(demo, connection, session_id)?,
        Command::Extension => run_extension(demo, connection, session_id).await?,
        Command::Fail => {
            return Err(acp::Error::internal_error().data("scripted failure from /fail"));
        }
        Command::Stop { reason } => run_stop(demo, connection, session_id, reason)?,
        Command::Unknown(name) => {
            send_text(
                demo,
                connection,
                session_id,
                format!("Unknown command `/{name}`. Run /help to list the demos."),
            )?;
            Turn::end_turn()
        }
        Command::Invalid { reason } => {
            send_text(
                demo,
                connection,
                session_id,
                format!("Invalid command: {reason}"),
            )?;
            Turn::end_turn()
        }
    };
    if demo.store().is_cancelled(session_id.0.as_ref()) || cancellation.is_cancelled() {
        turn.stop_reason = acp::StopReason::Cancelled;
        turn.usage = None;
    }
    Ok(turn)
}

fn prompt_text(prompt: &[acp::ContentBlock]) -> String {
    prompt
        .iter()
        .filter_map(|block| match block {
            acp::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn send_text(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    text: impl Into<String>,
) -> Result<(), acp::Error> {
    let message_id = demo.next_message_id("agent-message");
    demo.send_message_chunk(connection, session_id, &message_id, text)
}

fn run_help(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
            available_commands(),
        )),
    )?;
    send_text(demo, connection, session_id, demo::help_text())?;
    Ok(Turn::end_turn())
}

fn run_echo(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    config: &demo::SessionConfig,
    text: &str,
) -> Result<Turn, acp::Error> {
    if text.is_empty() {
        send_text(demo, connection, session_id, "Usage: /echo <text>")?;
        return Ok(Turn::end_turn());
    }
    if config.thinking {
        let thought_id = demo.next_message_id("thought");
        demo.send_thought_chunk(
            connection,
            session_id,
            &thought_id,
            "thinking is enabled for this session",
        )?;
    }
    let message_id = demo.next_message_id("agent-message");
    let split = text
        .char_indices()
        .find(|(index, _)| *index >= text.len() / 2)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let (head, tail) = text.split_at(split);
    demo.send_message_chunk(connection, session_id, &message_id, head)?;
    if !tail.is_empty() {
        demo.send_message_chunk(connection, session_id, &message_id, tail)?;
    }
    Ok(Turn::end_turn())
}

fn run_rich(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let resource_uri = demo
        .resources
        .sample_uri()
        .map_err(acp::Error::into_internal_error)?;
    let text_id = demo.next_message_id("agent-message");
    demo.send_message_chunk(
        connection,
        session_id,
        &text_id,
        "Rich content demo: text, image, audio, resource link, embedded resource.",
    )?;
    let image_id = demo.next_message_id("agent-message");
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(acp::ContentBlock::Image(acp::ImageContent::new(
                SAMPLE_IMAGE_BASE64,
                "image/png",
            )))
            .message_id(image_id),
        ),
    )?;
    let audio_id = demo.next_message_id("agent-message");
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(acp::ContentBlock::Audio(acp::AudioContent::new(
                SAMPLE_AUDIO_BASE64,
                "audio/wav",
            )))
            .message_id(audio_id),
        ),
    )?;
    let link_id = demo.next_message_id("agent-message");
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(acp::ContentBlock::ResourceLink(
                acp::ResourceLink::new("sample.md", resource_uri.clone())
                    .mime_type("text/markdown"),
            ))
            .message_id(link_id),
        ),
    )?;
    let resource_id = demo.next_message_id("agent-message");
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(acp::ContentBlock::Resource(acp::EmbeddedResource::new(
                acp::EmbeddedResourceResource::TextResourceContents(
                    acp::TextResourceContents::new(SAMPLE_RESOURCE, resource_uri)
                        .mime_type("text/markdown"),
                ),
            )))
            .message_id(resource_id),
        ),
    )?;
    Ok(Turn::end_turn())
}

fn run_thought(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let thought_id = demo.next_message_id("thought");
    demo.send_thought_chunk(
        connection,
        session_id,
        &thought_id,
        "The user asked for the thought demo, so I am emitting reasoning first.",
    )?;
    send_text(
        demo,
        connection,
        session_id,
        "Thought chunk sent; this is the answer chunk.",
    )?;
    Ok(Turn::end_turn())
}

fn run_tool(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    cwd: &std::path::Path,
) -> Result<Turn, acp::Error> {
    let tool_call_id = demo.next_tool_id();
    let path = cwd.join("README.md");
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new(tool_call_id.clone(), "Inspect the scripted workspace")
                .name("read_file")
                .kind(acp::ToolKind::Read)
                .status(acp::ToolCallStatus::Pending)
                .locations(vec![acp::ToolCallLocation::new(path.clone()).line(1)])
                .raw_input(serde_json::json!({"path": "README.md"})),
        ),
    )?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
            tool_call_id.clone(),
            acp::ToolCallUpdateFields::new()
                .status(acp::ToolCallStatus::InProgress)
                .content(vec![acp::ToolCallContent::from(
                    "Reading the scripted workspace...",
                )]),
        )),
    )?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
            tool_call_id,
            acp::ToolCallUpdateFields::new()
                .status(acp::ToolCallStatus::Completed)
                .content(vec![acp::ToolCallContent::Diff(
                    acp::Diff::new(cwd.join("README.md"), "# Updated by acp-test-agent\n")
                        .old_text("# Original heading\n"),
                )])
                .raw_output(serde_json::json!({"ok": true})),
        )),
    )?;
    send_text(
        demo,
        connection,
        session_id,
        "Tool call demo complete (pending -> in_progress -> completed).",
    )?;
    Ok(Turn::end_turn())
}

async fn run_permission(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let outcome = match client_ops::request_permission(demo, connection, session_id).await {
        Ok(outcome) => outcome,
        Err(error) => {
            send_text(
                demo,
                connection,
                session_id,
                format!("Permission request failed: {error}"),
            )?;
            return Ok(Turn::end_turn());
        }
    };
    let message = match outcome {
        PermissionDemo::Allowed => "Permission granted; the scripted tool ran.",
        PermissionDemo::Rejected => "Permission rejected; the scripted tool did not run.",
        PermissionDemo::Cancelled => return Ok(Turn::with(acp::StopReason::Cancelled)),
    };
    send_text(demo, connection, session_id, message)?;
    Ok(Turn::end_turn())
}

async fn run_elicitation(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let message = match client_ops::elicit_form(connection, session_id).await {
        Ok(action) => format!("Elicitation result: {}", describe_action(&action)),
        Err(error) => format!("Elicitation request failed: {error}"),
    };
    send_text(demo, connection, session_id, message)?;
    Ok(Turn::end_turn())
}

async fn run_elicitation_url(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let message = match client_ops::elicit_url(connection, session_id).await {
        Ok((action, elicitation_id)) => format!(
            "URL elicitation {elicitation_id}: {}; elicitation/complete sent.",
            describe_action(&action)
        ),
        Err(error) => format!("URL elicitation failed: {error}"),
    };
    send_text(demo, connection, session_id, message)?;
    Ok(Turn::end_turn())
}

fn describe_action(action: &acp::ElicitationAction) -> String {
    match action {
        acp::ElicitationAction::Accept(accepted) => {
            let fields = accepted
                .content
                .as_ref()
                .map(|content| content.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            format!("accepted (fields: {fields})")
        }
        acp::ElicitationAction::Decline => "declined".to_string(),
        acp::ElicitationAction::Cancel => "cancelled".to_string(),
        other => format!("other action: {other:?}"),
    }
}

async fn run_filesystem(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    cwd: &std::path::Path,
) -> Result<Turn, acp::Error> {
    let message = client_ops::exercise_filesystem(demo, connection, session_id, cwd).await?;
    send_text(
        demo,
        connection,
        session_id,
        format!("File-system demo: {message}"),
    )?;
    Ok(Turn::end_turn())
}

async fn run_terminal(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    match client_ops::exercise_terminal(demo, connection, session_id).await? {
        TerminalDemo::Unsupported => {
            send_text(
                demo,
                connection,
                session_id,
                "Client did not advertise terminal support.",
            )?;
        }
        TerminalDemo::Output {
            output, exit_code, ..
        } => {
            send_text(
                demo,
                connection,
                session_id,
                format!(
                    "Terminal demo complete (exit {exit_code:?}): {}",
                    output.trim()
                ),
            )?;
        }
    }
    Ok(Turn::end_turn())
}

async fn run_sleep(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    millis: u64,
    cancellation: &agent_client_protocol::RequestCancellation,
) -> Result<Turn, acp::Error> {
    if wait_cancellable(demo, session_id, millis, cancellation).await {
        send_text(
            demo,
            connection,
            session_id,
            "Sleep cancelled before it finished.",
        )?;
        return Ok(Turn::with(acp::StopReason::Cancelled));
    }
    send_text(
        demo,
        connection,
        session_id,
        format!("Slept for {millis} ms."),
    )?;
    Ok(Turn::end_turn())
}

async fn wait_cancellable(
    demo: &Demo,
    session_id: &acp::SessionId,
    millis: u64,
    cancellation: &agent_client_protocol::RequestCancellation,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(millis);
    loop {
        if cancellation.is_cancelled() {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let remaining = deadline.saturating_duration_since(now);
        tokio::time::sleep(remaining.min(Duration::from_millis(50))).await;
        // Check the store after sleeping so session/cancel can win the race
        // with a very short deadline.
        if demo.store().is_cancelled(session_id.0.as_ref()) {
            return true;
        }
    }
}

fn run_config(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    setting: Option<&str>,
) -> Result<Turn, acp::Error> {
    let result = {
        let mut store = demo.store();
        match store
            .get_mut(session_id.0.as_ref())
            .map(|session| session.config.apply_setting(setting))
        {
            Err(error) => Err(format!("Config demo failed: {error}")),
            Ok(Err(reason)) => Err(reason),
            Ok(Ok(())) => {
                let session = store
                    .get(session_id.0.as_ref())
                    .expect("the session was just updated");
                Ok(config_options(&session.config))
            }
        }
    };
    let options = match result {
        Ok(options) => options,
        Err(message) => {
            send_text(demo, connection, session_id, message)?;
            return Ok(Turn::end_turn());
        }
    };
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(options)),
    )?;
    send_text(demo, connection, session_id, "Config option updated.")?;
    Ok(Turn::end_turn())
}

fn run_commands(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let commands = available_commands();
    let count = commands.len();
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(commands)),
    )?;
    send_text(
        demo,
        connection,
        session_id,
        format!("Advertised {count} commands."),
    )?;
    Ok(Turn::end_turn())
}

fn run_usage(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::UsageUpdate(
            acp::UsageUpdate::new(1_200, 8_000).cost(acp::Cost::new(0.0042, "USD")),
        ),
    )?;
    send_text(
        demo,
        connection,
        session_id,
        "Usage update sent (1200/8000 tokens, $0.0042).",
    )?;
    Ok(Turn {
        stop_reason: acp::StopReason::EndTurn,
        usage: Some(
            acp::Usage::new(1_200, 900, 300)
                .thought_tokens(120)
                .cached_read_tokens(200),
        ),
    })
}

fn run_notice(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let supported = demo
        .client_capabilities()
        .and_then(|capabilities| capabilities.session)
        .is_some_and(|session| session.notices.is_some());
    if supported {
        demo.send_update(
            connection,
            session_id,
            acp::SessionUpdate::Notice(
                acp::Notice::new(acp::NoticeSeverity::Warning, "Scripted notice")
                    .description("This notice came from the ACP test agent."),
            ),
        )?;
        send_text(demo, connection, session_id, "Notice update sent.")?;
    } else {
        send_text(
            demo,
            connection,
            session_id,
            "Client did not advertise session notices; no notice update was sent.",
        )?;
    }
    Ok(Turn::end_turn())
}

fn run_compaction(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let supported = demo
        .client_capabilities()
        .and_then(|capabilities| capabilities.session)
        .is_some_and(|session| session.compaction.is_some());
    if supported {
        let compaction_id = acp::CompactionId::new("demo-compaction");
        demo.send_update(
            connection,
            session_id,
            acp::SessionUpdate::CompactionSummaryChunk(acp::CompactionSummaryChunk::new(
                compaction_id.clone(),
                acp::ContentBlock::from("Retained context from the scripted demo."),
            )),
        )?;
        demo.send_update(
            connection,
            session_id,
            acp::SessionUpdate::CompactionUpdate(acp::CompactionUpdate::new(
                compaction_id,
                acp::CompactionStatus::Completed,
            )),
        )?;
        send_text(demo, connection, session_id, "Compaction updates sent.")?;
    } else {
        send_text(
            demo,
            connection,
            session_id,
            "Client did not advertise compaction support; no compaction update was sent.",
        )?;
    }
    Ok(Turn::end_turn())
}

fn run_info(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    let title = "ACP test session (renamed)";
    let updated_at = demo::sessions::now();
    demo.store()
        .get_mut(session_id.0.as_ref())
        .map(|session| {
            session.title = title.to_string();
            session.updated_at = updated_at.clone();
        })
        .map_err(|error| acp::Error::invalid_params().data(error.to_string()))?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::SessionInfoUpdate(
            acp::SessionInfoUpdate::new()
                .title(title)
                .updated_at(updated_at),
        ),
    )?;
    send_text(demo, connection, session_id, "Session metadata updated.")?;
    Ok(Turn::end_turn())
}

async fn run_extension(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<Turn, acp::Error> {
    connection.send_notification(DemoNotice {
        message: "_demo/notice from acp-test-agent".to_string(),
    })?;
    let result = connection
        .send_request(EchoRequest {
            text: "ping from acp-test-agent".to_string(),
        })
        .block_task()
        .await;
    let message = match result {
        Ok(response) => format!(
            "Extension notification sent; _demo/echo replied with `{}`.",
            response.text
        ),
        Err(error) => format!("Extension notification sent; _demo/echo failed: {error}"),
    };
    send_text(demo, connection, session_id, message)?;
    Ok(Turn::end_turn())
}

fn run_stop(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    reason: DemoStopReason,
) -> Result<Turn, acp::Error> {
    if reason == DemoStopReason::Error {
        return Err(acp::Error::internal_error().data("/stop error requested a failed turn"));
    }
    let stop_reason = match reason {
        DemoStopReason::EndTurn => acp::StopReason::EndTurn,
        DemoStopReason::MaxTokens => acp::StopReason::MaxTokens,
        DemoStopReason::MaxTurnRequests => acp::StopReason::MaxTurnRequests,
        DemoStopReason::Refusal => acp::StopReason::Refusal,
        DemoStopReason::Cancelled => acp::StopReason::Cancelled,
        DemoStopReason::Error => unreachable!("handled above"),
    };
    send_text(
        demo,
        connection,
        session_id,
        format!("Turn ends with stop reason `{}`.", reason.label()),
    )?;
    Ok(Turn::with(stop_reason))
}

async fn run_plan(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    millis: u64,
    cancellation: &agent_client_protocol::RequestCancellation,
) -> Result<Turn, acp::Error> {
    let mut entries = vec![
        acp::PlanEntry::new(
            "Inspect the workspace",
            acp::PlanEntryPriority::High,
            acp::PlanEntryStatus::Completed,
        ),
        acp::PlanEntry::new(
            "Apply the scripted change",
            acp::PlanEntryPriority::Medium,
            acp::PlanEntryStatus::InProgress,
        ),
        acp::PlanEntry::new(
            "Verify the result",
            acp::PlanEntryPriority::Low,
            acp::PlanEntryStatus::Pending,
        ),
    ];
    publish_plan(demo, connection, session_id, entries.clone())?;
    for step in 1..entries.len() {
        if wait_cancellable(demo, session_id, millis, cancellation).await {
            remove_plan(demo, connection, session_id)?;
            send_text(demo, connection, session_id, "Plan cancelled.")?;
            return Ok(Turn::with(acp::StopReason::Cancelled));
        }
        entries[step].status = acp::PlanEntryStatus::Completed;
        if let Some(next) = entries.get_mut(step + 1) {
            next.status = acp::PlanEntryStatus::InProgress;
        }
        publish_plan(demo, connection, session_id, entries.clone())?;
    }
    send_text(
        demo,
        connection,
        session_id,
        "Plan complete. Use `/plan clear` to remove it.",
    )?;
    Ok(Turn::end_turn())
}

/// # Errors
/// Returns a transport or session-storage error if the plan cannot be published.
fn publish_plan(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
    entries: Vec<acp::PlanEntry>,
) -> Result<(), acp::Error> {
    let plan_capable = demo
        .client_capabilities()
        .is_some_and(|capabilities| capabilities.plan.is_some());
    let update = if plan_capable {
        acp::SessionUpdate::PlanUpdate(acp::PlanUpdate::new(acp::PlanUpdateContent::Items(
            acp::PlanItems::new("demo-plan", entries),
        )))
    } else {
        acp::SessionUpdate::Plan(acp::Plan::new(entries))
    };
    demo.send_update(connection, session_id, update)
}

/// # Errors
/// Returns a transport or session-storage error if the removal cannot be published.
fn remove_plan(
    demo: &Demo,
    connection: &ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), acp::Error> {
    let update = if demo
        .client_capabilities()
        .is_some_and(|capabilities| capabilities.plan.is_some())
    {
        acp::SessionUpdate::PlanRemoved(acp::PlanRemoved::new("demo-plan"))
    } else {
        acp::SessionUpdate::Plan(acp::Plan::new(Vec::new()))
    };
    demo.send_update(connection, session_id, update)
}
