use super::*;
use bas::service::pb::{self, bas_server::Bas};
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
        .accept_prompt(bas::model::PromptSubmission::new(v1::PromptRequest::new(
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
    let parsed = bas::authentication::elicitation::parse_request(&raw).expect("Codex form");
    let bas::authentication::elicitation::Mode::Form(form) = parsed.mode else {
        panic!("choice form expected");
    };
    assert!(form.fields.iter().any(|field| matches!(
        &field.kind,
        bas::authentication::elicitation::Kind::Enum { options }
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
            if let bas::model::EventKind::TurnFinished { result, .. } =
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
            if let bas::model::EventKind::Acp(v1::SessionUpdate::AgentMessageChunk(chunk)) =
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
