//! End-to-end tests for the v2 protocol implementation.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use acp_test_agent::demo::COMMANDS;
use acp_test_agent::extensions::{DemoNotice, EchoRequest, EchoResponse};
use agent_client_protocol::schema::{MaybeUndefined, ProtocolVersion, v2};
use agent_client_protocol::{Client, Error, Responder, V2ConnectionTo};
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::timeout;

use support::{UPDATE_TIMEOUT, agent, cwd};

async fn start_prompt(
    connection: &V2ConnectionTo<agent_client_protocol::Agent>,
    session_id: &v2::SessionId,
    text: &str,
) -> Result<v2::PromptResponse, Error> {
    connection
        .send_request(v2::PromptRequest::new(
            session_id.clone(),
            vec![text.into()],
        ))
        .block_task()
        .await
}

async fn next_update(
    updates: &mut tokio::sync::mpsc::UnboundedReceiver<v2::UpdateSessionNotification>,
    session_id: &v2::SessionId,
) -> v2::SessionUpdate {
    loop {
        let notification = timeout(UPDATE_TIMEOUT, updates.recv())
            .await
            .expect("session update arrived in time")
            .expect("connection stayed open");
        if &notification.session_id == session_id {
            return notification.update;
        }
    }
}

async fn wait_idle(
    updates: &mut tokio::sync::mpsc::UnboundedReceiver<v2::UpdateSessionNotification>,
    session_id: &v2::SessionId,
) -> (Vec<v2::SessionUpdate>, v2::StopReason) {
    let mut seen = Vec::new();
    loop {
        let update = next_update(updates, session_id).await;
        if let v2::SessionUpdate::StateUpdate(v2::StateUpdate::Idle(idle)) = &update {
            return (
                seen,
                idle.stop_reason
                    .clone()
                    .expect("the test agent always reports a stop reason"),
            );
        }
        seen.push(update);
    }
}

async fn drain(
    updates: &mut tokio::sync::mpsc::UnboundedReceiver<v2::UpdateSessionNotification>,
) -> Vec<v2::SessionUpdate> {
    let mut collected = Vec::new();
    while let Ok(Some(notification)) = timeout(Duration::from_millis(100), updates.recv()).await {
        collected.push(notification.update);
    }
    collected
}

fn text_of(update: &v2::SessionUpdate) -> Option<&str> {
    match update {
        v2::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
            v2::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn messages(updates: &[v2::SessionUpdate]) -> String {
    updates
        .iter()
        .filter_map(text_of)
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test]
async fn v2_lifecycle_and_feature_demos() -> Result<(), Box<dyn std::error::Error>> {
    let (update_tx, mut update_rx) = unbounded_channel();
    let (notice_tx, mut notice_rx) = unbounded_channel();
    let permission_seen = Arc::new(AtomicBool::new(false));
    let elicit_seen = Arc::new(AtomicBool::new(false));
    let permission_flag = permission_seen.clone();
    let elicit_flag = elicit_seen.clone();

    Client
        .v2()
        .name("v2-test-client")
        .on_receive_notification(
            async move |notification: v2::UpdateSessionNotification, _cx| {
                update_tx
                    .send(notification)
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
            async move |_request: v2::RequestPermissionRequest,
                        responder: Responder<v2::RequestPermissionResponse>,
                        _cx: agent_client_protocol::V2ConnectionTo<agent_client_protocol::Agent>| {
                permission_flag.store(true, Ordering::Relaxed);
                responder.respond(v2::RequestPermissionResponse::new(
                    v2::RequestPermissionOutcome::Selected(v2::SelectedPermissionOutcome::new(
                        "allow-once",
                    )),
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: v2::CreateElicitationRequest,
                        responder: Responder<v2::CreateElicitationResponse>,
                        _cx: agent_client_protocol::V2ConnectionTo<agent_client_protocol::Agent>| {
                elicit_flag.store(true, Ordering::Relaxed);
                responder.respond(v2::CreateElicitationResponse::new(
                    v2::ElicitationAction::Accept(
                        v2::ElicitationAcceptAction::new().content(BTreeMap::from([(
                            "project".to_string(),
                            v2::ElicitationContentValue::String("demo".to_string()),
                        )])),
                    ),
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: EchoRequest,
                        responder: Responder<EchoResponse>,
                        _cx: agent_client_protocol::V2ConnectionTo<agent_client_protocol::Agent>| {
                responder.respond(EchoResponse {
                    text: request.text.to_uppercase(),
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent(1), async move |cx| {
            let initialize = cx
                .send_request(v2::InitializeRequest::new(
                    ProtocolVersion::V2,
                    v2::Implementation::new("v2-test-client", "1.0.0"),
                ))
                .block_task()
                .await?;
            assert_eq!(initialize.protocol_version, ProtocolVersion::V2);
            assert!(initialize.capabilities.session.is_some());
            assert_eq!(initialize.auth_methods.len(), 1);

            let created = cx
                .send_request(v2::NewSessionRequest::new(cwd()))
                .block_task()
                .await?;
            let session_id = created.session_id.clone();
            assert_eq!(created.config_options.len(), 2);
            assert_eq!(created.available_commands.len(), COMMANDS.len());
            // v2 reports readiness as an idle state update, not the response.
            let ready = next_update(&mut update_rx, &session_id).await;
            assert!(matches!(
                ready,
                v2::SessionUpdate::StateUpdate(v2::StateUpdate::Idle(_))
            ));

            let accepted = start_prompt(&cx, &session_id, "/help").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert_eq!(messages(&updates), acp_test_agent::demo::help_text());
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::UserMessage(message)
                    if message.message_id == accepted.message_id
            )));
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::AvailableCommandsUpdate(_)
            )));

            start_prompt(&cx, &session_id, "/echo hello").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(messages(&updates).contains("hello"));

            start_prompt(&cx, &session_id, "/thought").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::AgentThoughtChunk(_)
            )));
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::AgentThought(_)
            )));

            start_prompt(&cx, &session_id, "/commands").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::AvailableCommandsUpdate(update)
                    if update.available_commands.len() == COMMANDS.len()
            )));

            start_prompt(&cx, &session_id, "/config model=deep").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::ConfigOptionUpdate(_)
            )));

            start_prompt(&cx, &session_id, "/elicit-url").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(messages(&updates).contains("elicitation/complete sent"));

            start_prompt(&cx, &session_id, "/sleep 10").await?;
            let (_, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);

            start_prompt(&cx, &session_id, "/rich").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::AgentMessage(message)
                    if matches!(&message.content, MaybeUndefined::Value(content) if content.len() == 5)
            )));
            let uri = updates.iter().find_map(|update| match update {
                v2::SessionUpdate::AgentMessage(message) => match &message.content {
                    MaybeUndefined::Value(content) => content.iter().find_map(|block| match block {
                        v2::ContentBlock::ResourceLink(resource) => Some(&resource.uri),
                        _ => None,
                    }),
                    _ => None,
                },
                _ => None,
            }).expect("resource link");
            let path = url::Url::parse(uri).expect("resource URL").to_file_path().expect("local resource");
            assert_eq!(std::fs::read_to_string(path).expect("open referenced file"), acp_test_agent::demo::SAMPLE_RESOURCE);

            start_prompt(&cx, &session_id, "/tool").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::ToolCallContentChunk(_)
            )));
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::ToolCallUpdate(update)
                    if update.status == MaybeUndefined::Value(v2::ToolCallStatus::Completed)
            )));

            start_prompt(&cx, &session_id, "/plan 0").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::PlanUpdate(_)
            )));
            let completed: Vec<_> = updates.iter().filter_map(|update| match update {
                v2::SessionUpdate::PlanUpdate(plan) => match &plan.plan {
                    v2::PlanUpdateContent::Items(plan) => Some(plan.entries.iter().filter(|entry| entry.status == v2::PlanEntryStatus::Completed).count()),
                    _ => None,
                },
                _ => None,
            }).collect();
            assert_eq!(completed, [1, 2, 3]);
            start_prompt(&cx, &session_id, "/plan clear").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(update, v2::SessionUpdate::PlanRemoved(_))));
            start_prompt(&cx, &session_id, "/plan 5000").await?;
            loop {
                if matches!(next_update(&mut update_rx, &session_id).await, v2::SessionUpdate::PlanUpdate(_)) { break; }
            }
            cx.send_notification(v2::CancelSessionNotification::new(session_id.clone()))?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::Cancelled);
            assert!(updates.iter().any(|update| matches!(update, v2::SessionUpdate::PlanRemoved(_))));
            assert!(!updates.iter().any(|update| matches!(update, v2::SessionUpdate::PlanUpdate(_))));

            start_prompt(&cx, &session_id, "/permission").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(permission_seen.load(Ordering::Relaxed));
            assert!(messages(&updates).contains("Permission granted"));

            start_prompt(&cx, &session_id, "/elicit").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(elicit_seen.load(Ordering::Relaxed));
            assert!(messages(&updates).contains("accepted"));

            start_prompt(&cx, &session_id, "/fs").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(messages(&updates).contains("removed client fs"));

            start_prompt(&cx, &session_id, "/terminal").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::TerminalUpdate(_)
            )));
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::TerminalOutputChunk(_)
            )));
            assert!(messages(&updates).contains("Agent-owned terminal demo"));

            start_prompt(&cx, &session_id, "/extension").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(messages(&updates).contains("PING FROM ACP-TEST-AGENT"));
            assert_eq!(
                notice_rx.recv().await.as_deref(),
                Some("_demo/notice from acp-test-agent")
            );

            start_prompt(&cx, &session_id, "/usage").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::UsageUpdate(_)
            )));

            start_prompt(&cx, &session_id, "/notice").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::Notice(_)
            )));

            start_prompt(&cx, &session_id, "/compact").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::CompactionUpdate(_)
            )));

            start_prompt(&cx, &session_id, "/info").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::SessionInfoUpdate(_)
            )));

            start_prompt(&cx, &session_id, "/stop max_tokens").await?;
            let (_, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::MaxTokens);

            start_prompt(&cx, &session_id, "/fail").await?;
            let (_, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert!(matches!(reason, v2::StopReason::Error(_)));

            start_prompt(&cx, &session_id, "/unknown").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(messages(&updates).contains("Unknown command"));

            let configured = cx
                .send_request(v2::SetSessionConfigOptionRequest::new(
                    session_id.clone(),
                    "model",
                    v2::SessionConfigOptionValue::id("deep"),
                ))
                .block_task()
                .await?;
            assert!(configured.config_options.iter().any(|option| matches!(
                &option.kind,
                v2::SessionConfigKind::Select(select)
                    if select.current_value.0.as_ref() == "deep"
            )));
            let updates = drain(&mut update_rx).await;
            assert!(updates.iter().any(|update| matches!(
                update,
                v2::SessionUpdate::ConfigOptionUpdate(_)
            )));

            // Cancellation: v2 already acknowledged the prompt, so the result
            // arrives as an idle update.
            let accepted = start_prompt(&cx, &session_id, "/sleep 5000").await?;
            assert!(!accepted.message_id.0.is_empty());
            loop {
                if matches!(
                    next_update(&mut update_rx, &session_id).await,
                    v2::SessionUpdate::StateUpdate(v2::StateUpdate::Running(_))
                ) {
                    break;
                }
            }
            cx.send_notification(v2::CancelSessionNotification::new(session_id.clone()))?;
            let (_, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::Cancelled);

            // Replay on resume: close first, then request history from start.
            cx.send_request(v2::CloseSessionRequest::new(session_id.clone()))
                .block_task()
                .await?;
            let resumed = cx
                .send_request(
                    v2::ResumeSessionRequest::new(session_id.clone(), cwd())
                        .replay_from(v2::ReplayFrom::Start(v2::ReplayFromStart::new())),
                )
                .block_task()
                .await?;
            assert_eq!(resumed.config_options.len(), 2);
            let replayed = drain(&mut update_rx).await;
            assert!(messages(&replayed).contains("hello") || messages(&replayed).contains("accepted"));
            start_prompt(&cx, &session_id, "/echo resumed").await?;
            let (updates, reason) = wait_idle(&mut update_rx, &session_id).await;
            assert_eq!(reason, v2::StopReason::EndTurn);
            assert!(messages(&updates).contains("resumed"));

            let fork = cx
                .send_request(v2::ForkSessionRequest::new(
                    session_id.clone(),
                    cwd(),
                ))
                .block_task()
                .await?;
            assert_ne!(fork.session_id, session_id);
            assert_eq!(fork.config_options.len(), 2);

            let first_page = cx
                .send_request(v2::ListSessionsRequest::new())
                .block_task()
                .await?;
            assert_eq!(first_page.sessions.len(), 1);
            assert!(first_page.next_cursor.is_some());
            let second_page = cx
                .send_request(
                    v2::ListSessionsRequest::new().cursor(first_page.next_cursor.clone()),
                )
                .block_task()
                .await?;
            assert_eq!(second_page.sessions.len(), 1);

            cx.send_request(v2::DeleteSessionRequest::new(fork.session_id))
                .block_task()
                .await?;
            cx.send_request(v2::DeleteSessionRequest::new(session_id))
                .block_task()
                .await?;
            let empty = cx
                .send_request(v2::ListSessionsRequest::new())
                .block_task()
                .await?;
            assert!(empty.sessions.is_empty());
            Ok(())
        })
        .await?;
    Ok(())
}
