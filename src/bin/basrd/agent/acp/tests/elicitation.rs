use super::*;
use basr::service::pb::{self, basr_server::Basr};
use futures::StreamExt;
use tonic::Request;

#[tokio::test]
async fn codex_shaped_async_choice_remains_answerable_after_the_turn_finishes() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = Arc::new(fixture_with_limits(
        "session_async_elicitation",
        &directory,
        Limits::default(),
    ));
    let session = registry.create(request()).await.expect("session");
    let service = crate::service::Service::new(registry.clone());
    let mut interactions = service
        .watch_elicitations(Request::new(pb::WatchElicitationsRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("elicitation subscription")
        .into_inner();
    let _ = interactions.next().await.expect("initial snapshot");
    let receipt = session
        .accept_prompt(basr::model::PromptSubmission::new(v1::PromptRequest::new(
            session.id().to_string(),
            vec!["ask asynchronously".into()],
        )))
        .await
        .expect("prompt");
    let snapshot = tokio::time::timeout(TIMEOUT, interactions.next())
        .await
        .expect("pending deadline")
        .expect("pending snapshot")
        .expect("valid snapshot");
    let pending = snapshot.pending.into_iter().next().expect("choice form");
    let raw: serde_json::Value = pending.request.expect("request").decode().expect("form");
    let parsed = basr::authentication::elicitation::parse_request(&raw).expect("Codex form");
    let basr::authentication::elicitation::Mode::Form(form) = parsed.mode else {
        panic!("choice form expected");
    };
    assert!(form.fields.iter().any(|field| matches!(
        &field.kind,
        basr::authentication::elicitation::Kind::Enum { options }
            if options.iter().map(|option| option.value.as_str()).collect::<Vec<_>>()
                == ["inspect", "implement"]
    )));
    let mut events = session
        .try_attach(AttachOptions {
            cursor: Some(receipt.cursor),
            replay: 0,
        })
        .await
        .expect("turn events");
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let basr::model::EventKind::TurnFinished { result, .. } =
                events.recv().await.expect("event").kind
            {
                assert_eq!(result.expect("result").stop_reason, StopReason::EndTurn);
                break;
            }
        }
    })
    .await
    .expect("the question does not block the turn");
    assert_eq!(session.meta().status, SessionStatus::Idle);
    assert!(!directory.path().join("pid.elicitation").exists());
    let answer = json!({
        "action": "accept",
        "content": {"workflow": "inspect", "workflow_note": "保留输入草稿"}
    });
    service
        .respond_elicitation(Request::new(pb::RespondElicitationRequest {
            session: session.id().to_string(),
            request_id: pending.request_id,
            response: Some((&answer).into()),
        }))
        .await
        .expect("the idle session still accepts the answer");
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let basr::model::EventKind::Acp(v1::SessionUpdate::AgentMessageChunk(chunk)) =
                events.recv().await.expect("answer event").kind
                && let ContentBlock::Text(text) = chunk.content
                && text.text == "Received async answer"
            {
                break;
            }
        }
    })
    .await
    .expect("out-of-turn follow-up notification");
    let reply: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("pid.elicitation")).expect("answer received"),
    )
    .expect("response JSON");
    assert_eq!(reply["id"], "async-form");
    assert_eq!(reply["result"], answer);
    drop(interactions);
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}

#[tokio::test]
async fn active_acp_session_routes_original_form_and_receives_user_response() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = Arc::new(fixture_with_limits(
        "session_elicitation",
        &directory,
        Limits::default(),
    ));
    let session = registry.create(request()).await.expect("session");
    let service = crate::service::Service::new(registry.clone());
    let mut interactions = service
        .watch_elicitations(Request::new(pb::WatchElicitationsRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("elicitation subscription")
        .into_inner();
    let _ = interactions
        .next()
        .await
        .expect("initial snapshot")
        .expect("valid snapshot");
    let receipt = session
        .accept_prompt(basr::model::PromptSubmission::new(v1::PromptRequest::new(
            session.id().to_string(),
            vec!["configure project".into()],
        )))
        .await
        .expect("prompt");
    let snapshot = tokio::time::timeout(TIMEOUT, interactions.next())
        .await
        .expect("pending deadline")
        .expect("pending snapshot")
        .expect("valid snapshot");
    let pending = snapshot.pending.into_iter().next().expect("form");
    let raw: serde_json::Value = pending
        .request
        .expect("request")
        .decode()
        .expect("raw form");
    assert_eq!(raw["sessionId"], "fixture");
    assert_eq!(raw["_meta"]["future"], "preserved");
    assert!(!directory.path().join("pid.elicitation").exists());
    assert!(
        service
            .respond_elicitation(Request::new(pb::RespondElicitationRequest {
                session: session.id().to_string(),
                request_id: pending.request_id.clone(),
                response: Some((&json!({"action":"accept","content":{}})).into())
            }))
            .await
            .is_err()
    );
    service
        .respond_elicitation(Request::new(pb::RespondElicitationRequest {
            session: session.id().to_string(),
            request_id: pending.request_id,
            response: Some((&json!({"action":"accept","content":{"project":"basr"}})).into()),
        }))
        .await
        .expect("valid answer");
    let mut events = session
        .try_attach(AttachOptions {
            cursor: Some(receipt.cursor),
            replay: 0,
        })
        .await
        .expect("turn events");
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let basr::model::EventKind::TurnFinished { result, .. } =
                events.recv().await.expect("event").kind
            {
                assert_eq!(
                    result.expect("turn finished").stop_reason,
                    StopReason::EndTurn
                );
                break;
            }
        }
    })
    .await
    .expect("completion deadline");
    let reply: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("pid.elicitation")).expect("agent response"),
    )
    .expect("response JSON");
    assert_eq!(reply["id"], "session-form");
    assert_eq!(
        reply["result"],
        json!({"action":"accept","content":{"project":"basr"}})
    );
    drop(interactions);
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}

#[tokio::test]
async fn cancelling_a_turn_clears_its_waiting_form_and_sends_cancel_to_agent() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = Arc::new(fixture_with_limits(
        "session_elicitation",
        &directory,
        Limits::default(),
    ));
    let session = registry.create(request()).await.expect("session");
    let service = crate::service::Service::new(registry.clone());
    let mut interactions = service
        .watch_elicitations(Request::new(pb::WatchElicitationsRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect("subscription")
        .into_inner();
    let _ = interactions
        .next()
        .await
        .expect("initial snapshot")
        .expect("valid snapshot");
    session
        .accept_prompt(basr::model::PromptSubmission::new(v1::PromptRequest::new(
            session.id().to_string(),
            vec!["configure project".into()],
        )))
        .await
        .expect("prompt");
    let snapshot = tokio::time::timeout(TIMEOUT, interactions.next())
        .await
        .expect("pending deadline")
        .expect("pending snapshot")
        .expect("valid snapshot");
    let pending = snapshot
        .pending
        .first()
        .expect("waiting form")
        .request_id
        .clone();
    service
        .cancel(Request::new(
            (&v1::CancelNotification::new(session.id().to_string())).into(),
        ))
        .await
        .expect("turn cancellation");
    let cleared = tokio::time::timeout(TIMEOUT, interactions.next())
        .await
        .expect("cancel deadline")
        .expect("cleared snapshot")
        .expect("valid snapshot");
    assert!(cleared.pending.is_empty());
    assert!(
        service
            .respond_elicitation(Request::new(pb::RespondElicitationRequest {
                session: session.id().to_string(),
                request_id: pending,
                response: Some((&json!({"action":"accept","content":{"project":"late"}})).into())
            }))
            .await
            .is_err()
    );
    let reply: serde_json::Value = tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Ok(bytes) = std::fs::read(directory.path().join("pid.elicitation"))
                && let Ok(reply) = serde_json::from_slice(&bytes)
            {
                break reply;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("agent cancellation response");
    assert_eq!(reply["result"], json!({"action":"cancel"}));
    drop(interactions);
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}
