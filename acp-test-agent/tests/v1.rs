//! End-to-end tests for the v1 protocol implementation.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use acp_test_agent::demo::COMMANDS;
use acp_test_agent::extensions::{DemoNotice, EchoRequest, EchoResponse};
use agent_client_protocol::schema::{ProtocolVersion, v1};
use agent_client_protocol::{Client, ConnectionTo, Error, Responder};
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::timeout;

use support::{UPDATE_TIMEOUT, agent, cwd};

async fn prompt(
    connection: &ConnectionTo<agent_client_protocol::Agent>,
    session_id: &v1::SessionId,
    text: &str,
) -> Result<v1::PromptResponse, Error> {
    connection
        .send_request(v1::PromptRequest::new(
            session_id.clone(),
            vec![text.into()],
        ))
        .block_task()
        .await
}

async fn drain(
    updates: &mut tokio::sync::mpsc::UnboundedReceiver<v1::SessionUpdate>,
) -> Vec<v1::SessionUpdate> {
    let mut collected = Vec::new();
    while let Ok(Some(update)) = timeout(Duration::from_millis(100), updates.recv()).await {
        collected.push(update);
    }
    collected
}

fn text_of(update: &v1::SessionUpdate) -> Option<&str> {
    match update {
        v1::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
            v1::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn messages(updates: &[v1::SessionUpdate]) -> String {
    updates
        .iter()
        .filter_map(text_of)
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test]
async fn v1_lifecycle_and_feature_demos() -> Result<(), Box<dyn std::error::Error>> {
    let (update_tx, mut update_rx) = unbounded_channel();
    let (notice_tx, mut notice_rx) = unbounded_channel();
    let permission_seen = Arc::new(AtomicBool::new(false));
    let elicit_seen = Arc::new(AtomicBool::new(false));

    let permission_flag = permission_seen.clone();
    let elicit_flag = elicit_seen.clone();
    Client
        .builder()
        .name("v1-test-client")
        .on_receive_notification(
            async move |notification: v1::SessionNotification, _cx| {
                update_tx
                    .send(notification.update)
                    .map_err(Error::into_internal_error)
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_notification(
            async move |notification: DemoNotice, _cx| {
                notice_tx
                    .send(notification.message)
                    .map_err(Error::into_internal_error)
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |_request: v1::RequestPermissionRequest,
                        responder: Responder<v1::RequestPermissionResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                permission_flag.store(true, Ordering::Relaxed);
                responder.respond(v1::RequestPermissionResponse::new(
                    v1::RequestPermissionOutcome::Selected(v1::SelectedPermissionOutcome::new(
                        "allow-once",
                    )),
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::CreateElicitationRequest,
                        responder: Responder<v1::CreateElicitationResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                elicit_flag.store(true, Ordering::Relaxed);
                responder.respond(v1::CreateElicitationResponse::new(
                    v1::ElicitationAction::Accept(v1::ElicitationAcceptAction::new().content(
                        BTreeMap::from([(
                            "project".to_string(),
                            v1::ElicitationContentValue::String("demo".to_string()),
                        )]),
                    )),
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::ReadTextFileRequest,
                        responder: Responder<v1::ReadTextFileResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(v1::ReadTextFileResponse::new("file contents\n"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::WriteTextFileRequest,
                        responder: Responder<v1::WriteTextFileResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(v1::WriteTextFileResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::CreateTerminalRequest,
                        responder: Responder<v1::CreateTerminalResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(v1::CreateTerminalResponse::new("terminal-1"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::TerminalOutputRequest,
                        responder: Responder<v1::TerminalOutputResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(v1::TerminalOutputResponse::new("terminal output", false))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::WaitForTerminalExitRequest,
                        responder: Responder<v1::WaitForTerminalExitResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(v1::WaitForTerminalExitResponse::new(
                    v1::TerminalExitStatus::new().exit_code(0),
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::KillTerminalRequest,
                        responder: Responder<v1::KillTerminalResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(v1::KillTerminalResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v1::ReleaseTerminalRequest,
                        responder: Responder<v1::ReleaseTerminalResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(v1::ReleaseTerminalResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: EchoRequest,
                        responder: Responder<EchoResponse>,
                        _cx: ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(EchoResponse {
                    text: request.text.to_uppercase(),
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent(1), async move |cx| {
            let capabilities = v1::ClientCapabilities::new()
                .fs(v1::FileSystemCapabilities::new()
                    .read_text_file(true)
                    .write_text_file(true))
                .terminal(true)
                .plan(v1::PlanCapabilities::new())
                .session(
                    v1::ClientSessionCapabilities::new()
                        .notices(v1::NoticeCapabilities::new())
                        .compaction(v1::CompactionCapabilities::new()),
                );
            let initialize = cx
                .send_request(
                    v1::InitializeRequest::new(ProtocolVersion::V1)
                        .client_capabilities(capabilities),
                )
                .block_task()
                .await?;
            assert_eq!(initialize.protocol_version, ProtocolVersion::V1);
            assert!(initialize.agent_capabilities.load_session);
            assert_eq!(initialize.auth_methods.len(), 1);

            let created = cx
                .send_request(v1::NewSessionRequest::new(cwd()))
                .block_task()
                .await?;
            let session_id = created.session_id.clone();
            let modes = created.modes.as_ref().expect("v1 sessions expose modes");
            assert_eq!(modes.current_mode_id.0.as_ref(), "ask");
            assert_eq!(modes.available_modes.len(), 3);
            assert_eq!(created.config_options.as_ref().map(Vec::len), Some(2));

            let response = prompt(&cx, &session_id, "/help").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("ACP test agent demos"));
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::AvailableCommandsUpdate(_)))
            );

            let response = prompt(&cx, &session_id, "/thought").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::AgentThoughtChunk(_)))
            );

            let response = prompt(&cx, &session_id, "/rich").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(updates.iter().any(|update| matches!(
                update,
                v1::SessionUpdate::AgentMessageChunk(chunk)
                    if matches!(chunk.content, v1::ContentBlock::Image(_))
            )));
            assert!(updates.iter().any(|update| matches!(
                update,
                v1::SessionUpdate::AgentMessageChunk(chunk)
                    if matches!(chunk.content, v1::ContentBlock::Resource(_))
            )));

            let response = prompt(&cx, &session_id, "/commands").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(updates.iter().any(|update| matches!(
                update,
                v1::SessionUpdate::AvailableCommandsUpdate(update)
                    if update.available_commands.len() == COMMANDS.len()
            )));

            let response = prompt(&cx, &session_id, "/config model=deep").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::ConfigOptionUpdate(_)))
            );

            let response = prompt(&cx, &session_id, "/elicit-url").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("elicitation/complete sent"));

            let response = prompt(&cx, &session_id, "/sleep 10").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let _ = drain(&mut update_rx).await;

            let response = prompt(&cx, &session_id, "/tool").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::ToolCall(_)))
            );
            assert!(updates.iter().any(|update| matches!(
                update,
                v1::SessionUpdate::ToolCallUpdate(update)
                    if update.fields.status == Some(v1::ToolCallStatus::Completed)
            )));

            let response = prompt(&cx, &session_id, "/plan").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::Plan(_)))
            );
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::PlanUpdate(_)))
            );

            let response = prompt(&cx, &session_id, "/permission").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            assert!(permission_seen.load(Ordering::Relaxed));
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("Permission granted"));

            let response = prompt(&cx, &session_id, "/elicit").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            assert!(elicit_seen.load(Ordering::Relaxed));
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("accepted"));

            let response = prompt(&cx, &session_id, "/fs").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("read back 14 bytes"));

            let response = prompt(&cx, &session_id, "/terminal").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("Terminal demo complete"));

            let response = prompt(&cx, &session_id, "/extension").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("PING FROM ACP-TEST-AGENT"));
            assert_eq!(
                notice_rx.recv().await.as_deref(),
                Some("_demo/notice from acp-test-agent")
            );

            let response = prompt(&cx, &session_id, "/usage").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let usage = response.usage.expect("v1 /usage sets turn usage");
            assert_eq!(usage.total_tokens, 1_200);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::UsageUpdate(_)))
            );

            let response = prompt(&cx, &session_id, "/notice").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::Notice(_)))
            );

            let response = prompt(&cx, &session_id, "/compact").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::CompactionUpdate(_)))
            );

            let response = prompt(&cx, &session_id, "/info").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::SessionInfoUpdate(_)))
            );

            let response = prompt(&cx, &session_id, "/stop max_tokens").await?;
            assert_eq!(response.stop_reason, v1::StopReason::MaxTokens);
            let _ = drain(&mut update_rx).await;

            // Errors are reported as JSON-RPC errors for v1 prompts.
            assert!(prompt(&cx, &session_id, "/fail").await.is_err());
            let _ = drain(&mut update_rx).await;

            let response = prompt(&cx, &session_id, "/unknown").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let updates = drain(&mut update_rx).await;
            assert!(messages(&updates).contains("Unknown command"));

            let configured = cx
                .send_request(v1::SetSessionConfigOptionRequest::new(
                    session_id.clone(),
                    "model",
                    v1::SessionConfigOptionValue::value_id("deep"),
                ))
                .block_task()
                .await?;
            assert!(configured.config_options.iter().any(|option| matches!(
                &option.kind,
                v1::SessionConfigKind::Select(select)
                    if select.current_value.0.as_ref() == "deep"
            )));
            let updates = drain(&mut update_rx).await;
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, v1::SessionUpdate::ConfigOptionUpdate(_)))
            );

            cx.send_request(v1::SetSessionModeRequest::new(session_id.clone(), "code"))
                .block_task()
                .await?;
            let updates = drain(&mut update_rx).await;
            assert!(updates.iter().any(|update| matches!(
                update,
                v1::SessionUpdate::CurrentModeUpdate(mode)
                    if mode.current_mode_id.0.as_ref() == "code"
            )));

            // Record history, close, then load it back.
            prompt(&cx, &session_id, "/echo history").await?;
            let _ = drain(&mut update_rx).await;
            cx.send_request(v1::CloseSessionRequest::new(session_id.clone()))
                .block_task()
                .await?;
            let loaded = cx
                .send_request(v1::LoadSessionRequest::new(session_id.clone(), cwd()))
                .block_task()
                .await?;
            assert!(loaded.modes.is_some());
            let replayed = drain(&mut update_rx).await;
            assert!(messages(&replayed).contains("history"));

            cx.send_request(v1::ResumeSessionRequest::new(session_id.clone(), cwd()))
                .block_task()
                .await?;
            let response = prompt(&cx, &session_id, "/echo resumed").await?;
            assert_eq!(response.stop_reason, v1::StopReason::EndTurn);
            let _ = drain(&mut update_rx).await;

            let fork = cx
                .send_request(v1::ForkSessionRequest::new(session_id.clone(), cwd()))
                .block_task()
                .await?;
            assert_ne!(fork.session_id, session_id);
            assert!(fork.modes.is_some());
            assert_eq!(fork.config_options.as_ref().map(Vec::len), Some(2));

            // Pagination: page size 1, two sessions exist after the fork.
            let first_page = cx
                .send_request(v1::ListSessionsRequest::new())
                .block_task()
                .await?;
            assert_eq!(first_page.sessions.len(), 1);
            assert!(first_page.next_cursor.is_some());
            let second_page = cx
                .send_request(v1::ListSessionsRequest::new().cursor(first_page.next_cursor.clone()))
                .block_task()
                .await?;
            assert_eq!(second_page.sessions.len(), 1);
            assert_ne!(
                first_page.sessions[0].session_id,
                second_page.sessions[0].session_id
            );

            // Cancellation is cooperative and observed by the worker.
            let prompt_cx = cx.clone();
            let prompt_id = session_id.clone();
            let pending =
                tokio::spawn(async move { prompt(&prompt_cx, &prompt_id, "/sleep 5000").await });
            tokio::time::sleep(Duration::from_millis(100)).await;
            cx.send_notification(v1::CancelNotification::new(session_id.clone()))?;
            let cancelled = timeout(UPDATE_TIMEOUT, pending)
                .await
                .expect("cancelled prompt responded in time")
                .expect("prompt task did not panic")
                .expect("cancelled prompt responds with a stop reason");
            assert_eq!(cancelled.stop_reason, v1::StopReason::Cancelled);

            cx.send_request(v1::DeleteSessionRequest::new(fork.session_id))
                .block_task()
                .await?;
            cx.send_request(v1::DeleteSessionRequest::new(session_id))
                .block_task()
                .await?;
            let empty = cx
                .send_request(v1::ListSessionsRequest::new())
                .block_task()
                .await?;
            assert!(empty.sessions.is_empty());
            Ok(())
        })
        .await?;
    Ok(())
}
