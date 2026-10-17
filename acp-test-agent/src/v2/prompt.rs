//! v2 prompt handling: accept immediately, then report output and idle state.

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v2 as acp;
use agent_client_protocol::{Client, Error, Responder, V2ConnectionTo};

use super::Demo;
use super::client_ops::{self, PermissionDemo};
use super::updates::{available_commands, config_options};
use crate::demo::commands::{self, Command, StopReason as DemoStopReason};
use crate::demo::{self, SAMPLE_AUDIO_BASE64, SAMPLE_IMAGE_BASE64, SAMPLE_RESOURCE};
use crate::extensions::{DemoNotice, EchoRequest};

/// Handle `session/prompt`.
///
/// In v2 the response acknowledges insertion into the conversation; output and
/// completion arrive later as session updates. Work is therefore always spawned,
/// and errors after acceptance become a failed idle state instead of an error
/// response.
pub(super) fn handle_prompt(
    demo: &Arc<Demo>,
    connection: V2ConnectionTo<Client>,
    request: acp::PromptRequest,
    responder: Responder<acp::PromptResponse>,
) -> Result<(), Error> {
    let session_id = request.session_id.clone();
    if let Err(error) = demo.store().begin_prompt(session_id.0.as_ref()) {
        return responder.respond_with_error(Error::invalid_params().data(error.to_string()));
    }
    let session_key = session_id.0.to_string();
    let message_id = demo.next_message_id("user-message");
    if let Err(error) = demo.send_update(
        &connection,
        &session_id,
        acp::SessionUpdate::UserMessage(
            acp::UserMessage::new(message_id.clone()).content(request.prompt.clone()),
        ),
    ) {
        demo.store().finish_prompt(&session_key);
        return responder.respond_with_error(error);
    }
    if let Err(error) = responder.respond(acp::PromptResponse::new(message_id)) {
        demo.store().finish_prompt(&session_key);
        return Err(error);
    }
    if let Err(error) = demo.send_running(&connection, &session_id) {
        demo.store().finish_prompt(&session_key);
        return Err(error);
    }

    let worker_demo = demo.clone();
    let worker_session_key = session_key.clone();
    if let Err(error) = connection.clone().spawn(async move {
        let reason = match run_turn(&worker_demo, &connection, &request).await {
            Ok(reason) => reason,
            Err(error) => acp::StopReason::Error(
                acp::ErrorStopReason::new()
                    .error(acp::Error::internal_error().data(error.to_string())),
            ),
        };
        worker_demo.store().finish_prompt(&worker_session_key);
        worker_demo.send_idle(&connection, &session_id, reason)
    }) {
        demo.store().finish_prompt(&session_key);
        return Err(error);
    }
    Ok(())
}

async fn run_turn(
    demo: &Arc<Demo>,
    connection: &V2ConnectionTo<Client>,
    request: &acp::PromptRequest,
) -> Result<acp::StopReason, Error> {
    let session_id = &request.session_id;
    let command = commands::parse(&prompt_text(&request.prompt));
    let (cwd, config) = {
        let store = demo.store();
        let session = store
            .get(session_id.0.as_ref())
            .map_err(|error| Error::invalid_params().data(error.to_string()))?;
        (session.cwd.clone(), session.config.clone())
    };
    let reason = match command {
        Command::Help => {
            run_help(demo, connection, session_id)?;
            acp::StopReason::EndTurn
        }
        Command::Echo { text } => {
            run_echo(demo, connection, session_id, &config, &text)?;
            acp::StopReason::EndTurn
        }
        Command::Rich => {
            run_rich(demo, connection, session_id)?;
            acp::StopReason::EndTurn
        }
        Command::Thought => {
            run_thought(demo, connection, session_id)?;
            acp::StopReason::EndTurn
        }
        Command::Tool => {
            run_tool(demo, connection, session_id, &cwd)?;
            acp::StopReason::EndTurn
        }
        Command::Plan => {
            run_plan(demo, connection, session_id)?;
            acp::StopReason::EndTurn
        }
        Command::Permission => {
            match client_ops::request_permission(demo, connection, session_id).await {
                Ok(PermissionDemo::Allowed) => {
                    send_text(
                        demo,
                        connection,
                        session_id,
                        "Permission granted; the scripted tool ran.",
                    )?;
                    acp::StopReason::EndTurn
                }
                Ok(PermissionDemo::Rejected) => {
                    send_text(
                        demo,
                        connection,
                        session_id,
                        "Permission rejected; the scripted tool did not run.",
                    )?;
                    acp::StopReason::EndTurn
                }
                Ok(PermissionDemo::Cancelled) => acp::StopReason::Cancelled,
                Err(error) => {
                    send_text(
                        demo,
                        connection,
                        session_id,
                        format!("Permission request failed: {error}"),
                    )?;
                    acp::StopReason::EndTurn
                }
            }
        }
        Command::Elicitation => {
            let message = match client_ops::elicit_form(connection, session_id).await {
                Ok(action) => format!("Elicitation result: {}", describe_action(&action)),
                Err(error) => format!("Elicitation request failed: {error}"),
            };
            send_text(demo, connection, session_id, message)?;
            acp::StopReason::EndTurn
        }
        Command::ElicitationUrl => {
            let message = match client_ops::elicit_url(connection, session_id).await {
                Ok(action) => format!(
                    "URL elicitation: {}; elicitation/complete sent.",
                    describe_action(&action)
                ),
                Err(error) => format!("URL elicitation failed: {error}"),
            };
            send_text(demo, connection, session_id, message)?;
            acp::StopReason::EndTurn
        }
        Command::FileSystem => {
            send_text(
                demo,
                connection,
                session_id,
                "ACP v2 removed client fs/* callbacks; use /terminal for agent-owned execution.",
            )?;
            acp::StopReason::EndTurn
        }
        Command::Terminal => {
            let (terminal_id, output) =
                client_ops::exercise_terminal(demo, connection, session_id, &cwd)?;
            let tool_call_id = demo.next_tool_id();
            demo.send_update(
                connection,
                session_id,
                acp::SessionUpdate::ToolCallUpdate(
                    acp::ToolCallUpdate::new(tool_call_id)
                        .title("Run an agent-owned terminal")
                        .kind(acp::ToolKind::Execute)
                        .status(acp::ToolCallStatus::Completed)
                        .content(vec![acp::ToolCallContent::Terminal(acp::Terminal::new(
                            terminal_id,
                        ))]),
                ),
            )?;
            send_text(
                demo,
                connection,
                session_id,
                format!("Agent-owned terminal demo: {}", output.trim()),
            )?;
            acp::StopReason::EndTurn
        }
        Command::Sleep { millis } => {
            if wait_cancellable(demo, session_id, millis).await {
                send_text(
                    demo,
                    connection,
                    session_id,
                    "Sleep cancelled before it finished.",
                )?;
                acp::StopReason::Cancelled
            } else {
                send_text(
                    demo,
                    connection,
                    session_id,
                    format!("Slept for {millis} ms."),
                )?;
                acp::StopReason::EndTurn
            }
        }
        Command::Config { setting } => {
            run_config(demo, connection, session_id, setting.as_deref())?;
            acp::StopReason::EndTurn
        }
        Command::Commands => {
            run_commands(demo, connection, session_id)?;
            acp::StopReason::EndTurn
        }
        Command::Usage => {
            run_usage(demo, connection, session_id)?;
            acp::StopReason::EndTurn
        }
        Command::Notice => {
            demo.send_update(
                connection,
                session_id,
                acp::SessionUpdate::Notice(
                    acp::Notice::new(acp::NoticeSeverity::Warning, "Scripted notice")
                        .description("This notice came from the ACP test agent."),
                ),
            )?;
            send_text(demo, connection, session_id, "Notice update sent.")?;
            acp::StopReason::EndTurn
        }
        Command::Compaction => {
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
            acp::StopReason::EndTurn
        }
        Command::Info => {
            run_info(demo, connection, session_id)?;
            acp::StopReason::EndTurn
        }
        Command::Extension => {
            run_extension(demo, connection, session_id).await?;
            acp::StopReason::EndTurn
        }
        Command::Fail => {
            return Err(Error::internal_error().data("scripted failure from /fail"));
        }
        Command::Stop { reason } => {
            if reason == DemoStopReason::Error {
                return Err(Error::internal_error().data("/stop error requested a failed turn"));
            }
            let label = reason.label();
            let reason = match reason {
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
                format!("Turn ends with stop reason `{label}`."),
            )?;
            reason
        }
        Command::Unknown(name) => {
            send_text(
                demo,
                connection,
                session_id,
                format!("Unknown command `/{name}`. Run /help to list the demos."),
            )?;
            acp::StopReason::EndTurn
        }
        Command::Invalid { reason } => {
            send_text(
                demo,
                connection,
                session_id,
                format!("Invalid command: {reason}"),
            )?;
            acp::StopReason::EndTurn
        }
    };
    if demo.store().is_cancelled(session_id.0.as_ref()) {
        return Ok(acp::StopReason::Cancelled);
    }
    Ok(reason)
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
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
    text: impl Into<String>,
) -> Result<(), Error> {
    let message_id = demo.next_message_id("agent-message");
    demo.send_message_chunk(connection, session_id, &message_id, text.into())
}

fn run_help(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
            available_commands(),
        )),
    )?;
    send_text(demo, connection, session_id, demo::help_text())
}

fn run_echo(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
    config: &demo::SessionConfig,
    text: &str,
) -> Result<(), Error> {
    if text.is_empty() {
        return send_text(demo, connection, session_id, "Usage: /echo <text>");
    }
    if config.thinking {
        let thought_id = demo.next_message_id("thought");
        demo.send_update(
            connection,
            session_id,
            acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
                "thinking is enabled for this session".into(),
                thought_id,
            )),
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
    Ok(())
}

fn run_rich(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
    let message_id = demo.next_message_id("agent-message");
    demo.send_message_chunk(
        connection,
        session_id,
        &message_id,
        "Rich content demo: text, image, audio, resource link, embedded resource.",
    )?;
    let content = vec![
        acp::ContentBlock::from(
            "Rich content demo: text, image, audio, resource link, embedded resource.",
        ),
        acp::ContentBlock::Image(acp::ImageContent::new(SAMPLE_IMAGE_BASE64, "image/png")),
        acp::ContentBlock::Audio(acp::AudioContent::new(SAMPLE_AUDIO_BASE64, "audio/wav")),
        acp::ContentBlock::ResourceLink(
            acp::ResourceLink::new("sample.md", "file:///acp-test-agent/sample.md")
                .mime_type("text/markdown"),
        ),
        acp::ContentBlock::Resource(acp::EmbeddedResource::new(
            acp::EmbeddedResourceResource::TextResourceContents(
                acp::TextResourceContents::new(SAMPLE_RESOURCE, "file:///acp-test-agent/sample.md")
                    .mime_type("text/markdown"),
            ),
        )),
    ];
    // The snapshot patches the chunked message with the same ID. Clients must
    // replace accumulated content rather than append a second copy.
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AgentMessage(acp::AgentMessage::new(message_id).content(content)),
    )
}

fn run_thought(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
    let thought_id = demo.next_message_id("thought");
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
            "The user asked for the thought demo, so I am emitting reasoning first.".into(),
            thought_id.clone(),
        )),
    )?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::AgentThought(
            acp::AgentThought::new(thought_id)
                .content(vec!["Reasoning complete; answering now.".into()]),
        ),
    )?;
    send_text(
        demo,
        connection,
        session_id,
        "Thought chunk sent; this is the answer chunk.",
    )
}

fn run_tool(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
    cwd: &std::path::Path,
) -> Result<(), Error> {
    let tool_call_id = demo.next_tool_id();
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(tool_call_id.clone())
                .title("Inspect the scripted workspace")
                .name("read_file")
                .kind(acp::ToolKind::Read)
                .status(acp::ToolCallStatus::Pending)
                .locations(vec![
                    acp::ToolCallLocation::new(cwd.join("README.md")).line(1),
                ])
                .raw_input(serde_json::json!({"path": "README.md"})),
        ),
    )?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(tool_call_id.clone())
                .title("Reading the scripted workspace")
                .status(acp::ToolCallStatus::InProgress),
        ),
    )?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCallContentChunk(acp::ToolCallContentChunk::new(
            tool_call_id.clone(),
            "Reading the scripted workspace...",
        )),
    )?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(tool_call_id)
                .status(acp::ToolCallStatus::Completed)
                .content(vec![acp::ToolCallContent::Diff(acp::Diff::new(vec![
                    acp::DiffChange::modify(cwd.join("README.md")),
                    acp::DiffChange::move_file(cwd.join("old-name.txt"), cwd.join("new-name.txt")),
                ]))])
                .raw_output(serde_json::json!({"ok": true})),
        ),
    )?;
    send_text(
        demo,
        connection,
        session_id,
        "Tool call demo complete (upserts, content chunk, structured multi-file diff).",
    )
}

fn run_plan(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
    let entries = vec![
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
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::PlanUpdate(acp::PlanUpdate::new(acp::PlanUpdateContent::Items(
            acp::PlanItems::new("demo-plan", entries),
        ))),
    )?;
    send_text(demo, connection, session_id, "Plan update published.")
}

fn run_config(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
    setting: Option<&str>,
) -> Result<(), Error> {
    let result = {
        let mut store = demo.store();
        match store
            .get_mut(session_id.0.as_ref())
            .map(|session| demo::apply_config_setting(&mut session.config, setting))
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
        Err(message) => return send_text(demo, connection, session_id, message),
    };
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(options)),
    )?;
    send_text(demo, connection, session_id, "Config option updated.")
}

fn run_commands(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
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
    )
}

fn run_usage(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
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
    )
}

fn run_info(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
    let title = "ACP test session (renamed)";
    let updated_at = demo::sessions::now();
    demo.store()
        .get_mut(session_id.0.as_ref())
        .map(|session| {
            session.title = title.to_string();
            session.updated_at = updated_at.clone();
        })
        .map_err(|error| Error::invalid_params().data(error.to_string()))?;
    demo.send_update(
        connection,
        session_id,
        acp::SessionUpdate::SessionInfoUpdate(
            acp::SessionInfoUpdate::new()
                .title(title)
                .updated_at(updated_at),
        ),
    )?;
    send_text(demo, connection, session_id, "Session metadata updated.")
}

async fn run_extension(
    demo: &Demo,
    connection: &V2ConnectionTo<Client>,
    session_id: &acp::SessionId,
) -> Result<(), Error> {
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
    send_text(demo, connection, session_id, message)
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

async fn wait_cancellable(demo: &Demo, session_id: &acp::SessionId, millis: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(millis);
    loop {
        if demo.store().is_cancelled(session_id.0.as_ref()) {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let remaining = deadline.saturating_duration_since(now);
        tokio::time::sleep(remaining.min(Duration::from_millis(50))).await;
    }
}
