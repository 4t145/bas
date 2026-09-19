use std::{sync::Arc, time::Duration};

use basr::model::{ContentBlock, Owner, SessionStatus};
use serde_json::json;

use super::*;
use crate::{
    agent::{AgentSpec, Catalog, Command, LocalHost},
    session::{AttachError, AttachOptions, CreateError, CreateRequest, Limits, Registry},
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn update(value: serde_json::Value) -> v1::SessionUpdate {
    serde_json::from_value(value).expect("valid ACP fixture")
}

#[cfg(unix)]
fn fixture(mode: &str, directory: &tempfile::TempDir, timeout: Duration) -> Registry {
    let mut spec = AgentSpec::acp(
        "fixture",
        Command::new(
            "sh",
            [
                "-c".to_owned(),
                include_str!("assets/fixture.sh").to_owned(),
            ],
        ),
    );
    spec.env.insert("BASR_FIXTURE_MODE".into(), mode.into());
    spec.env.insert(
        "BASR_FIXTURE_PID".into(),
        directory.path().join("pid").to_string_lossy().into_owned(),
    );
    let mut catalog = Catalog::new();
    catalog
        .insert(spec, Arc::new(Driver::new()))
        .expect("fixture driver");
    Registry::new(
        Arc::new(catalog),
        Arc::new(LocalHost::default()),
        Limits {
            startup_timeout: timeout,
            ..Limits::default()
        },
    )
}

#[cfg(unix)]
fn request() -> CreateRequest {
    CreateRequest {
        agent: "fixture".into(),
        owner: Owner::current(),
        cwd: None,
        title: None,
    }
}

#[cfg(unix)]
async fn wait_for_exit(directory: &tempfile::TempDir) {
    use nix::{sys::signal::kill, unistd::Pid};
    let pid: i32 = std::fs::read_to_string(directory.path().join("pid"))
        .expect("child pid")
        .trim()
        .parse()
        .expect("numeric pid");
    tokio::time::timeout(TIMEOUT, async {
        while kill(Pid::from_raw(pid), None).is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned ACP child exits");
}

#[cfg(unix)]
#[tokio::test]
async fn acp_finishes_and_notifies_after_the_last_client_disconnects() {
    use basr::{
        model::EventKind,
        service::{pb, pb::basr_server::Basr},
    };
    use futures::StreamExt;
    use tonic::Request;

    let directory = tempfile::tempdir().expect("fixture directory");
    let (completions, mut completed) = mpsc::channel(1);
    let registry =
        Arc::new(fixture("wait_release", &directory, TIMEOUT).with_completions(completions));
    let session = registry
        .create(request())
        .await
        .expect("ACP session starts");
    let service = crate::service::Service::new(registry.clone());
    let attach = pb::AttachRequest {
        session: session.id().to_string(),
        cursor: None,
        replay: 100,
    };
    let stream = service
        .attach(Request::new(attach.clone()))
        .await
        .expect("attach")
        .into_inner();
    let prompt = v1::PromptRequest::new(session.id().to_string(), vec!["hello".into()]);
    service
        .prompt(Request::new((&prompt).into()))
        .await
        .expect("prompt accepted");
    tokio::time::timeout(TIMEOUT, async {
        while !directory.path().join("pid.prompt").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("backend starts working");
    assert_eq!(session.meta().status, SessionStatus::Working);
    drop(stream);
    assert_eq!(session.subscriber_count(), 0);
    // The backend may finish only after every client has disconnected.
    std::fs::write(directory.path().join("pid.release"), "").expect("release backend");
    let completion = tokio::time::timeout(TIMEOUT, completed.recv())
        .await
        .expect("background completion does not require reconnecting")
        .expect("completion notification");
    assert_eq!(completion.session.id, session.id());
    assert_eq!(
        completion.result.expect("successful turn").stop_reason,
        StopReason::EndTurn
    );
    assert_eq!(session.meta().status, SessionStatus::Idle);

    let mut replay = service
        .attach(Request::new(attach))
        .await
        .expect("reattach")
        .into_inner();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let event = basr::model::Event::try_from(
                replay
                    .next()
                    .await
                    .expect("replay stream")
                    .expect("replay event"),
            )
            .expect("decode event");
            if matches!(event.kind, EventKind::TurnFinished { .. }) {
                break;
            }
        }
    })
    .await
    .expect("reconnect replays the already completed turn");
    assert!(
        completed.try_recv().is_err(),
        "replay must not notify again"
    );
    drop(replay);
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
#[tokio::test]
async fn acp_process_streams_native_content_and_is_reaped_on_shutdown() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = fixture("normal", &directory, TIMEOUT);
    let session = registry
        .create(request())
        .await
        .expect("ACP session starts");
    session
        .set_title("local title")
        .expect("ACP sessions can be renamed locally");
    let mut attachment = session
        .attach(AttachOptions {
            replay: 0,
            cursor: None,
        })
        .expect("attach");
    session
        .prompt(vec![ContentBlock::from("hello")])
        .expect("prompt accepted");
    let events = tokio::time::timeout(TIMEOUT, async {
        let mut events = Vec::new();
        loop {
            let event = attachment.recv().await.expect("event");
            let done = matches!(event.kind, basr::model::EventKind::TurnFinished { .. });
            events.push(event);
            if done {
                return events;
            }
        }
    })
    .await
    .expect("ACP turn finishes");
    assert!(events.iter().any(|event| matches!(&event.kind, basr::model::EventKind::Acp(v1::SessionUpdate::AgentThoughtChunk(chunk)) if matches!(&chunk.content, ContentBlock::Text(text) if text.text == "thinking"))));
    assert!(events.iter().any(|event| matches!(&event.kind, basr::model::EventKind::Acp(v1::SessionUpdate::AgentMessageChunk(chunk)) if matches!(&chunk.content, ContentBlock::Text(text) if text.text == "answer"))));
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
#[tokio::test]
async fn acp_disconnects_and_prompt_errors_fail_the_session() {
    for mode in ["idle_exit", "prompt_error"] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let registry = fixture(mode, &directory, TIMEOUT);
        let session = registry
            .create(request())
            .await
            .expect("ACP session starts");
        let mut attachment = session
            .attach(AttachOptions {
                replay: 0,
                cursor: None,
            })
            .expect("attach");
        if mode == "prompt_error" {
            session
                .prompt(vec![ContentBlock::from("fail")])
                .expect("prompt queued");
        }
        tokio::time::timeout(TIMEOUT, async {
            while let Ok(event) = attachment.recv().await {
                if matches!(event.kind, basr::model::EventKind::Failed { .. }) {
                    return;
                }
            }
            panic!("failure must be visible before closure");
        })
        .await
        .expect("failure observed");
        assert_eq!(session.meta().status, SessionStatus::Failed);
        assert_eq!(attachment.recv().await, Err(AttachError::Closed));
        registry.shutdown().await;
        wait_for_exit(&directory).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn startup_timeout_and_request_cancellation_kill_the_partial_process() {
    for cancel in [false, true] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let registry = Arc::new(fixture(
            "hang_start",
            &directory,
            if cancel {
                TIMEOUT
            } else {
                Duration::from_secs(1)
            },
        ));
        let creator = Arc::clone(&registry);
        let task = tokio::spawn(async move { creator.create(request()).await });
        tokio::time::timeout(TIMEOUT, async {
            while !directory.path().join("pid").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("process started");
        if cancel {
            task.abort();
            assert!(task.await.err().expect("request cancelled").is_cancelled());
        } else {
            assert!(matches!(
                task.await.expect("request returned"),
                Err(CreateError::Timeout)
            ));
        }
        wait_for_exit(&directory).await;
        assert!(registry.list().await.is_empty());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_before_and_after_prompt_dispatch_completes_the_turn() {
    for immediate in [true, false] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let registry = fixture("wait_cancel", &directory, TIMEOUT);
        let session = registry
            .create(request())
            .await
            .expect("ACP session starts");
        let mut attachment = session
            .attach(AttachOptions {
                replay: 0,
                cursor: None,
            })
            .expect("attach");
        session
            .prompt(vec![ContentBlock::from("wait")])
            .expect("prompt");
        if !immediate {
            wait_for_prompt(&directory).await;
        }
        let cancel = v1::CancelNotification::new(session.id().to_string())
            .meta(serde_json::from_value::<v1::Meta>(json!({"cancel":"metadata"})).expect("meta"));
        session.cancel_acp(cancel).expect("cancel requested");
        tokio::time::timeout(TIMEOUT, async {
            loop {
                if let basr::model::EventKind::TurnFinished { result, .. } =
                    attachment.recv().await.expect("event").kind
                {
                    assert_eq!(
                        result.expect("cancel result").stop_reason,
                        StopReason::Cancelled
                    );
                    return;
                }
            }
        })
        .await
        .expect("cancellation completes");
        if !immediate {
            let recorded: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(directory.path().join("pid.cancel"))
                    .expect("cancel request"),
            )
            .expect("JSON");
            assert_eq!(recorded["params"]["sessionId"], "fixture");
            assert_eq!(recorded["params"]["_meta"]["cancel"], "metadata");
        }
        registry.shutdown().await;
        wait_for_exit(&directory).await;
    }
}

#[cfg(unix)]
async fn wait_for_prompt(directory: &tempfile::TempDir) {
    tokio::time::timeout(TIMEOUT, async {
        while !directory.path().join("pid.prompt").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("peer received the prompt");
}

#[cfg(unix)]
#[tokio::test]
async fn a_peer_that_ignores_cancellation_is_failed_and_stopped() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = fixture("ignore_cancel", &directory, TIMEOUT);
    let session = registry.create(request()).await.expect("ACP starts");
    let mut attachment = session
        .attach(AttachOptions {
            replay: 0,
            cursor: None,
        })
        .expect("attach");
    session
        .prompt(vec![ContentBlock::from("wait")])
        .expect("prompt");
    wait_for_prompt(&directory).await;
    session.cancel().expect("cancel requested");
    tokio::time::timeout(CANCEL_TIMEOUT + TIMEOUT, async {
        loop {
            if let basr::model::EventKind::Failed { message } =
                attachment.recv().await.expect("failure event").kind
            {
                assert!(message.contains("after cancellation"));
                return;
            }
        }
    })
    .await
    .expect("cancellation has a deadline");
    assert_eq!(session.meta().status, SessionStatus::Failed);
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
#[tokio::test]
async fn native_acp_requests_options_and_rich_updates_cross_the_service_and_replay() {
    use basr::{
        model::{Event, EventKind},
        service::{acp_error, pb, pb::basr_server::Basr},
    };
    use futures::StreamExt;
    use tonic::Request;
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = Arc::new(fixture("rich", &directory, TIMEOUT));
    let session = registry.create(request()).await.expect("ACP session");
    let service = crate::service::Service::new(registry.clone());
    let id = session.id().to_string();
    let meta = session.meta();
    assert!(
        meta.initialization
            .expect("capabilities")
            .agent_capabilities
            .prompt_capabilities
            .image
    );
    assert_eq!(meta.acp.expect("ACP session").session_id.to_string(), id);

    let config = v1::SetSessionConfigOptionRequest::new(id.clone(), "model", "deep")
        .meta(serde_json::from_value::<v1::Meta>(json!({"request":"metadata"})).expect("meta"));
    let configured: v1::SetSessionConfigOptionResponse = tokio::time::timeout(
        TIMEOUT,
        service.set_config_option(Request::new((&config).into())),
    )
    .await
    .expect("config completes")
    .expect("config response")
    .into_inner()
    .decode()
    .expect("ACP config response");
    assert_eq!(
        configured.meta.as_ref().expect("response meta")["fixture"],
        "config"
    );
    assert_eq!(
        session.meta().acp.expect("state").config_options,
        Some(configured.config_options)
    );
    let received: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("pid.config")).expect("recorded config"),
    )
    .expect("JSON");
    assert_eq!(received["params"]["sessionId"], "fixture");
    assert_eq!(received["params"]["_meta"]["request"], "metadata");

    let invalid = v1::SetSessionConfigOptionRequest::new(id.clone(), "model", "invalid");
    let status = service
        .set_config_option(Request::new((&invalid).into()))
        .await
        .expect_err("invalid choice");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    let error = acp_error(&status).expect("native ACP error");
    assert_eq!(error.code, v1::ErrorCode::InvalidParams);
    assert_eq!(error.data.expect("error data")["configId"], "model");
    assert_eq!(session.meta().status, SessionStatus::Idle);

    let mode = v1::SetSessionModeRequest::new(id.clone(), "code");
    service
        .set_mode(Request::new((&mode).into()))
        .await
        .expect("set mode");
    assert_eq!(
        session
            .meta()
            .acp
            .expect("state")
            .modes
            .expect("modes")
            .current_mode_id
            .to_string(),
        "code"
    );

    let before = session
        .page(None, 100)
        .expect("history")
        .events
        .last()
        .expect("event")
        .cursor
        .raw();
    let mut stream = service
        .attach(Request::new(pb::AttachRequest {
            session: id.clone(),
            cursor: Some(before),
            replay: 0,
        }))
        .await
        .expect("attach")
        .into_inner();
    let content: Vec<v1::ContentBlock> = serde_json::from_value(json!([
        {"type":"text","text":"hello","_meta":{"text":"metadata"}},
        {"type":"image","data":"aW1hZ2U=","mimeType":"image/png"},
        {"type":"resource","resource":{"uri":"file:///input.txt","text":"input","mimeType":"text/plain"}}
    ])).expect("native content");
    let prompt = v1::PromptRequest::new(id.clone(), content.clone())
        .meta(serde_json::from_value::<v1::Meta>(json!({"prompt":"metadata"})).expect("meta"));
    service
        .prompt(Request::new((&prompt).into()))
        .await
        .expect("prompt accepted");
    let events = tokio::time::timeout(TIMEOUT, async {
        let mut events = Vec::new();
        loop {
            let event = Event::try_from(stream.next().await.expect("stream").expect("event"))
                .expect("decode");
            let done = matches!(event.kind, EventKind::TurnFinished { .. });
            events.push(event);
            if done {
                break events;
            }
        }
    })
    .await
    .expect("turn completes");
    let recorded: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("pid.request")).expect("recorded request"),
    )
    .expect("JSON");
    let received: v1::PromptRequest =
        serde_json::from_value(recorded["params"].clone()).expect("ACP prompt");
    assert_eq!(received.session_id.to_string(), "fixture");
    assert_eq!(received.prompt, content);
    assert_eq!(received.meta, prompt.meta);
    assert!(events.iter().any(|event| matches!(&event.kind, EventKind::Acp(v1::SessionUpdate::AgentMessageChunk(chunk)) if matches!(&chunk.content, ContentBlock::Image(image) if image.meta.is_some()))));
    assert!(events.iter().any(|event| matches!(&event.kind, EventKind::Acp(v1::SessionUpdate::ToolCall(tool)) if tool.locations[0].line == Some(7) && !tool.content.is_empty())));
    assert!(
        matches!(&events.last().expect("finish").kind, EventKind::TurnFinished { result: Ok(response), .. } if response.meta.as_ref().expect("response meta")["fixture"] == "response")
    );
    let replay: Vec<Event> = service
        .load_history(Request::new(pb::LoadHistoryRequest {
            session: id,
            after: Some(before),
            limit: 100,
        }))
        .await
        .expect("history")
        .into_inner()
        .events
        .into_iter()
        .map(|event| event.try_into().expect("event"))
        .collect();
    assert_eq!(replay, events);
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}

#[tokio::test]
async fn metadata_notifications_outside_a_turn_are_preserved_and_foreign_sessions_are_rejected() {
    let (sender, mut receiver) = mpsc::channel(2);
    let mut state = State {
        session: Some("fixture".into()),
        active: None,
    };
    let update = update(
        json!({"sessionUpdate":"config_option_update","configOptions":[{"id":"enabled","name":"Enabled","type":"boolean","currentValue":true}],"_meta":{"revision":4}}),
    );
    state
        .on_notification(
            v1::SessionNotification::new("fixture", update.clone()),
            &sender,
        )
        .await
        .expect("idle metadata");
    assert_eq!(receiver.recv().await, Some(Update::Acp(update.clone())));
    assert!(
        state
            .on_notification(v1::SessionNotification::new("other", update), &sender)
            .await
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn updates_during_session_creation_override_the_initial_snapshot() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = fixture("early_update", &directory, TIMEOUT);
    let session = registry.create(request()).await.expect("session");
    let state = session.meta().acp.expect("ACP snapshot");
    assert_eq!(
        state.config_options.expect("options")[0].id.to_string(),
        "enabled"
    );
    assert_eq!(
        state.modes.expect("modes").current_mode_id.to_string(),
        "code"
    );
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}
