use std::{sync::Arc, time::Duration};

use bas::model::{ContentBlock, Owner, SessionStatus};
#[cfg(unix)]
use futures::future::BoxFuture;
use serde_json::json;

use super::*;
#[cfg(unix)]
use crate::storage::Storage;
use crate::{
    agent::{AgentSpec, Catalog, Command, LocalHost},
    session::{AttachError, AttachOptions, CreateError, CreateRequest, Limits, Registry},
};

const TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(unix)]
mod elicitation;
#[cfg(unix)]
mod fixture;
#[cfg(unix)]
mod history;
#[cfg(unix)]
mod permissions;
#[cfg(unix)]
mod subagents;

#[cfg(unix)]
struct RejectPromptStore {
    inner: crate::storage::Memory,
}

#[cfg(unix)]
impl crate::storage::Storage for RejectPromptStore {
    fn create(
        &self,
        meta: bas::model::SessionMeta,
        resume: Option<crate::storage::ResumeData>,
    ) -> BoxFuture<'_, Result<(), crate::storage::Error>> {
        self.inner.create(meta, resume)
    }

    fn commit<'a>(
        &'a self,
        meta: bas::model::SessionMeta,
        events: &'a [bas::model::Event],
        acceptance: Option<crate::storage::PromptAcceptance>,
    ) -> BoxFuture<'a, Result<(), crate::storage::Error>> {
        Box::pin(async move {
            if events
                .iter()
                .any(|event| matches!(event.kind, bas::model::EventKind::TurnStarted { .. }))
            {
                return Err(crate::storage::Error::InvalidCommit(
                    "injected prompt commit failure",
                ));
            }
            self.inner.commit(meta, events, acceptance).await
        })
    }

    fn acceptance(
        &self,
        session: bas::model::SessionId,
        turn: bas::model::TurnId,
    ) -> BoxFuture<'_, Result<Option<crate::storage::PromptAcceptance>, crate::storage::Error>>
    {
        self.inner.acceptance(session, turn)
    }

    fn save_resume(
        &self,
        id: SessionId,
        resume: Option<crate::storage::ResumeData>,
    ) -> BoxFuture<'_, Result<(), crate::storage::Error>> {
        self.inner.save_resume(id, resume)
    }

    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<crate::storage::StoredSession>, crate::storage::Error>> {
        self.inner.load(id)
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<bas::model::SessionMeta>, crate::storage::Error>> {
        self.inner.list()
    }

    fn page(
        &self,
        id: SessionId,
        after: Option<bas::model::Cursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<crate::storage::Page, crate::storage::Error>> {
        self.inner.page(id, after, limit)
    }

    fn tail(
        &self,
        id: SessionId,
        count: usize,
    ) -> BoxFuture<'_, Result<Vec<bas::model::Event>, crate::storage::Error>> {
        self.inner.tail(id, count)
    }

    fn delete(&self, id: SessionId) -> BoxFuture<'_, Result<bool, crate::storage::Error>> {
        self.inner.delete(id)
    }

    fn is_persistent(&self) -> bool {
        false
    }
}

#[cfg(unix)]
#[tokio::test]
async fn failed_prompt_commit_never_dispatches_to_the_acp_agent() {
    use std::num::NonZeroUsize;

    let directory = tempfile::tempdir().expect("temporary directory");
    let storage = Arc::new(RejectPromptStore {
        inner: crate::storage::Memory::new(NonZeroUsize::new(64).expect("nonzero capacity")),
    });
    let registry = Registry::with_storage(
        Arc::new(Catalog::from_specs([fixture_spec("load", &directory)]).expect("catalog")),
        Arc::new(LocalHost::default()),
        Limits::default(),
        Arc::clone(&storage) as Arc<dyn Storage>,
    );
    let session = registry.create(request()).await.expect("create session");
    let before = session.meta().revision;
    assert!(matches!(
        session
            .accept_prompt(bas::model::PromptSubmission::new(v1::PromptRequest::new(
                session.id().to_string(),
                vec!["must not run".into()],
            )))
            .await,
        Err(crate::session::SubmissionError::Unknown(
            crate::session::SessionError::Runtime(bas::model::RuntimeFailure::Persistence { .. })
        ))
    ));
    assert_eq!(session.meta().revision, before);
    assert_eq!(session.snapshot().status(), SessionStatus::Failed);
    assert!(registry.ensure_restored(&session).await.is_err());
    assert!(!directory.path().join("pid.prompt").exists());
    assert_eq!(
        storage
            .load(session.id())
            .await
            .expect("load")
            .expect("saved session")
            .meta
            .revision,
        before
    );
    registry.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn persisted_acp_session_loads_before_continuing_without_duplicating_history() {
    use std::os::unix::fs::PermissionsExt;

    use crate::storage::LocalDb;

    let directory = tempfile::tempdir().expect("temporary directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private database directory");
    let database = directory.path().join("sessions.sqlite");
    let open = || async {
        Registry::with_storage(
            Arc::new(
                Catalog::from_specs([fixture_spec("context", &directory)])
                    .expect("fixture catalog"),
            ),
            Arc::new(LocalHost::default()),
            Limits::default(),
            Arc::new(LocalDb::open(&database).await.expect("open storage")),
        )
    };

    let registry = open().await;
    let session = registry.create(request()).await.expect("create");
    let public_id = session.id();
    let receipt = session
        .accept_prompt(bas::model::PromptSubmission::new(v1::PromptRequest::new(
            public_id.to_string(),
            vec!["first".into()],
        )))
        .await
        .expect("first prompt");
    let mut attachment = session
        .try_attach(AttachOptions {
            replay: 0,
            cursor: Some(receipt.cursor),
        })
        .await
        .expect("attach first turn");
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if matches!(
                attachment.recv().await.expect("event").kind,
                bas::model::EventKind::TurnFinished { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("first turn finishes");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("pid.context")).expect("agent context"),
        "1\n"
    );
    let before = session.meta().revision;
    registry.shutdown().await;
    drop(attachment);
    drop(session);
    drop(registry);
    let stored = LocalDb::open(&database).await.expect("reopen storage");
    assert_eq!(
        stored
            .load(public_id)
            .await
            .expect("load")
            .expect("saved session")
            .resume
            .expect("resume data")
            .acp_session_id()
            .expect("ACP ID"),
        "fixture"
    );
    drop(stored);
    std::fs::remove_file(directory.path().join("pid.new")).expect("clear first session/new marker");

    let mut registry = open().await;
    registry.recover().await.expect("rebuild registry");
    let session = registry.get(public_id).await.expect("same public session");
    assert_eq!(session.meta().revision, before);
    registry
        .ensure_restored(&session)
        .await
        .expect("load original ACP session");
    assert!(
        !directory.path().join("pid.new").exists(),
        "restore must not create another ACP session"
    );
    assert!(directory.path().join("pid.load").exists());
    assert_eq!(
        session.meta().revision,
        before,
        "session/load history replay is not appended a second time"
    );
    let receipt = session
        .accept_prompt(bas::model::PromptSubmission::new(v1::PromptRequest::new(
            public_id.to_string(),
            vec!["second".into()],
        )))
        .await
        .expect("continue original session");
    let mut resumed = session
        .try_attach(AttachOptions {
            replay: 0,
            cursor: Some(receipt.cursor),
        })
        .await
        .expect("attach second turn");
    let mut continued_context = false;
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let event = resumed.recv().await.expect("event");
            if matches!(
                event.kind,
                bas::model::EventKind::Acp(v1::SessionUpdate::AgentMessageChunk(_))
            ) && serde_json::to_string(&event.kind)
                .expect("event JSON")
                .contains("turn 2 after 1")
            {
                continued_context = true;
            }
            if matches!(event.kind, bas::model::EventKind::TurnFinished { .. }) {
                break;
            }
        }
    })
    .await
    .expect("restored turn finishes");
    assert!(
        continued_context,
        "the restored agent used its prior-turn state"
    );
    let sent: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("pid.request")).expect("forwarded prompt"),
    )
    .expect("prompt JSON");
    assert_eq!(sent["params"]["sessionId"], "fixture");
    assert!(sent.to_string().contains("second"));
    assert!(session.meta().revision > before);
    registry.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_during_acp_load_cannot_reinstall_a_backend() {
    use std::os::unix::fs::PermissionsExt;

    use crate::storage::LocalDb;

    let directory = tempfile::tempdir().expect("temporary directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private database directory");
    let database = directory.path().join("sessions.sqlite");
    let open = || async {
        Registry::with_storage(
            Arc::new(
                Catalog::from_specs([fixture_spec("hang_load", &directory)])
                    .expect("fixture catalog"),
            ),
            Arc::new(LocalHost::default()),
            Limits::default(),
            Arc::new(LocalDb::open(&database).await.expect("open storage")),
        )
    };
    let registry = open().await;
    let session = registry.create(request()).await.expect("create");
    let id = session.id();
    registry.shutdown().await;
    drop(session);
    drop(registry);

    let mut registry = open().await;
    registry.recover().await.expect("recover");
    let registry = Arc::new(registry);
    let session = registry.get(id).await.expect("saved session");
    let restoring = {
        let registry = Arc::clone(&registry);
        let session = Arc::clone(&session);
        tokio::spawn(async move { registry.ensure_restored(&session).await })
    };
    tokio::time::timeout(TIMEOUT, async {
        while !directory.path().join("pid.load").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("ACP session/load started");
    tokio::time::timeout(TIMEOUT, registry.shutdown())
        .await
        .expect("shutdown must join the restoring backend");
    assert!(
        tokio::time::timeout(TIMEOUT, restoring)
            .await
            .expect("restore request finishes")
            .expect("restore task did not panic")
            .is_err()
    );
    assert!(matches!(
        session
            .accept_prompt(bas::model::PromptSubmission::new(v1::PromptRequest::new(
                id.to_string(),
                vec!["after shutdown".into()],
            )))
            .await,
        Err(crate::session::SubmissionError::Rejected(
            bas::model::PromptRejection {
                kind: bas::model::PromptRejectionKind::Unavailable,
                ..
            }
        ))
    ));
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
#[tokio::test]
async fn session_operation_auth_errors_keep_native_details_without_mutating_session() {
    use bas::service::{acp_error, pb::bas_server::Bas};
    use tonic::Request;

    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = Arc::new(fixture("auth_operation", &directory, TIMEOUT));
    let session = registry.create(request()).await.expect("ready session");
    let service = crate::service::Service::new(registry.clone());
    let before = session.meta();
    let id = session.id().to_string();
    let config = v1::SetSessionConfigOptionRequest::new(id.clone(), "model", "deep");
    let mode = v1::SetSessionModeRequest::new(id, "code");

    let errors = [
        tokio::time::timeout(
            TIMEOUT,
            service.set_config_option(Request::new((&config).into())),
        )
        .await
        .expect("config response deadline")
        .expect_err("configuration requires authentication"),
        tokio::time::timeout(TIMEOUT, service.set_mode(Request::new((&mode).into())))
            .await
            .expect("mode response deadline")
            .expect_err("mode requires authentication"),
    ];
    for status in errors {
        assert_eq!(status.code(), tonic::Code::Unauthenticated);
        let error = acp_error(&status).expect("native ACP error");
        assert_eq!(error.code, v1::ErrorCode::AuthRequired);
        assert_eq!(error.message, "Sign in required");
        assert_eq!(error.data, Some(json!({"reason": "expired"})));
    }
    assert_eq!(
        session.meta(),
        before,
        "a native request rejection is not a session lifecycle transition"
    );
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
#[tokio::test]
async fn auth_required_startup_preserves_acp_details_without_creating_session() {
    use bas::service::{pb, pb::bas_server::Bas};
    for (mode, method) in [("auth_session", None), ("auth_data", Some("specific"))] {
        let directory = tempfile::tempdir().unwrap();
        let registry = Arc::new(fixture(mode, &directory, TIMEOUT));
        let service = crate::service::Service::new(registry.clone());
        let error = service
            .create_session(tonic::Request::new(pb::CreateSessionRequest {
                agent: "fixture".into(),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::Unauthenticated);
        let acp: v1::Error = serde_json::from_slice(error.details()).unwrap();
        assert_eq!(acp.code, agent_client_protocol::ErrorCode::AuthRequired);
        if let Some(method) = method {
            assert_eq!(
                acp.data.as_ref().expect("agent method list")["authMethods"][0]["id"],
                method
            );
            let raw: serde_json::Value =
                serde_json::from_slice(error.details()).expect("original ACP error");
            assert_eq!(raw["futureError"], json!({"preserved":true}));
        } else {
            assert!(
                acp.data
                    .as_ref()
                    .and_then(|data| data.get("authMethods"))
                    .is_none(),
                "initialization methods are separate from native errors"
            );
        }
        assert!(!error.message().contains("SECRET"));
        assert!(registry.list().await.is_empty());
        assert!(!directory.path().join("pid.prompt").exists());
        wait_for_exit(&directory).await;
    }
    let directory = tempfile::tempdir().unwrap();
    let registry = Arc::new(fixture("reject_session", &directory, TIMEOUT));
    let Err(error) = registry.create(request()).await else {
        panic!("rejected session unexpectedly started")
    };
    assert!(matches!(
        error,
        CreateError::Agent(crate::agent::Error::Acp(_))
    ));
    assert!(registry.list().await.is_empty());
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
fn fixture(mode: &str, directory: &tempfile::TempDir, timeout: Duration) -> Registry {
    fixture_with_limits(
        mode,
        directory,
        Limits {
            startup_timeout: timeout,
            ..Limits::default()
        },
    )
}

#[cfg(unix)]
fn fixture_with_limits(mode: &str, directory: &tempfile::TempDir, limits: Limits) -> Registry {
    let spec = fixture_spec(mode, directory);
    let mut catalog = Catalog::new();
    catalog
        .insert(spec, Arc::new(Driver::new()))
        .expect("fixture driver");
    Registry::new(Arc::new(catalog), Arc::new(LocalHost::default()), limits)
}

#[cfg(unix)]
fn fixture_spec(mode: &str, directory: &tempfile::TempDir) -> AgentSpec {
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
    spec.cancellation_grace = Duration::from_millis(50);
    spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
        directory.path().join("pid").to_string_lossy().into_owned(),
    );
    spec
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
async fn acp_disconnects_and_corrupt_responses_fail_the_session() {
    for mode in [
        "idle_exit",
        "prompt_disconnect",
        "malformed_prompt",
        "corrupt_prompt",
    ] {
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
            .await
            .expect("attach");
        if mode != "idle_exit" {
            session
                .prompt(vec![ContentBlock::from("fail")])
                .await
                .expect("prompt queued");
        }
        tokio::time::timeout(TIMEOUT, async {
            while let Ok(event) = attachment.recv().await {
                if matches!(event.kind, bas::model::EventKind::Failed { .. }) {
                    return;
                }
            }
            panic!("failure must be visible before closure");
        })
        .await
        .unwrap_or_else(|_| panic!("failure observed for {mode}"));
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
async fn cancellation_before_worker_dispatch_has_no_native_result() {
    let directory = tempfile::tempdir().expect("directory");
    let (updates, mut inbox) = mpsc::channel(16);
    let public_id = SessionId::new();
    let mut agent = Driver::new()
        .launch(LaunchRequest {
            session: public_id,
            spec: Arc::new(fixture_spec("wait_cancel", &directory)),
            cwd: std::env::current_dir().expect("cwd"),
            additional_directories: Vec::new(),
            host: Arc::new(LocalHost::default()),
            updates,
        })
        .await
        .expect("launch");
    let turn = TurnId::new();
    // No yield between enqueue and cancellation: the prompt worker cannot dispatch.
    agent
        .prompt(
            turn,
            v1::PromptRequest::new(public_id.to_string(), vec!["wait".into()]),
        )
        .expect("prompt");
    agent
        .cancel(v1::CancelNotification::new(public_id.to_string()))
        .expect("cancel");
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Update::Turn {
                turn: completed,
                update,
            } = inbox.recv().await.expect("update")
            {
                assert_eq!(completed, turn);
                assert_eq!(update, TurnUpdate::CancelledBeforeDispatch);
                break;
            }
        }
    })
    .await
    .expect("interrupted");
    assert!(!directory.path().join("pid.request").exists());
    agent.shutdown().await;
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
#[tokio::test]
async fn native_acp_requests_options_and_rich_updates_cross_the_service_and_replay() {
    use bas::{
        model::{Event, EventKind},
        service::{acp_error, pb, pb::bas_server::Bas},
    };
    use futures::StreamExt;
    use tonic::Request;
    let directory = tempfile::tempdir().expect("fixture directory");
    let registry = Arc::new(fixture("rich", &directory, TIMEOUT));
    let session = registry.create(request()).await.expect("ACP session");
    let service = crate::service::Service::new(registry.clone());
    let id = session.id().to_string();
    let meta = session.meta();
    let initialize: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("pid.initialize"))
            .expect("recorded initialization"),
    )
    .expect("initialization JSON");
    assert_eq!(initialize["params"]["protocolVersion"], 1);
    assert_eq!(
        initialize["params"]["clientCapabilities"]["plan"],
        json!({})
    );
    assert_eq!(
        initialize["params"]["clientCapabilities"]["session"]["compaction"],
        json!({})
    );
    assert_eq!(
        initialize["params"]["clientCapabilities"]["session"]["notices"],
        json!({})
    );
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
        .prompt(Request::new(
            (&bas::model::PromptSubmission::new(prompt.clone())).into(),
        ))
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
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::Acp(v1::SessionUpdate::PlanUpdate(update))
            if update.meta.as_ref().expect("plan metadata")["fixture"] == "plan"
    )));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::Acp(v1::SessionUpdate::CompactionSummaryChunk(chunk))
            if chunk.compaction_id.to_string() == "fixture-compaction"
    )));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::Acp(v1::SessionUpdate::CompactionUpdate(update))
            if update.error.is_null()
                && update.meta.value().expect("compaction metadata")["fixture"] == "compaction"
    )));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::Acp(v1::SessionUpdate::Notice(notice))
            if event.cursor == bas::model::Cursor::START
                && notice.severity == v1::NoticeSeverity::Warning
                && notice.title == "Provider retry"
    )));
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
    let ordered: Vec<_> = events
        .into_iter()
        .filter(|event| !event.is_transient())
        .collect();
    assert!(
        ordered
            .windows(2)
            .all(|pair| pair[0].cursor.next() == pair[1].cursor)
    );
    assert_eq!(replay, ordered);
    registry.shutdown().await;
    wait_for_exit(&directory).await;
}

#[cfg(unix)]
fn config_value(session: &crate::session::Session) -> serde_json::Value {
    serde_json::to_value(
        session
            .meta()
            .acp
            .expect("ACP metadata")
            .config_options
            .expect("options"),
    )
    .expect("options JSON")[0]["currentValue"]
        .clone()
}

#[cfg(unix)]
#[tokio::test]
async fn configuration_commit_survives_cancelled_waiter_and_later_notification() {
    for cancel_waiter in [true, false] {
        let directory = tempfile::tempdir().expect("fixture directory");
        let registry = fixture("config_notification", &directory, TIMEOUT);
        let session = registry.create(request()).await.expect("session");
        let mut events = session
            .attach(AttachOptions {
                replay: 0,
                cursor: None,
            })
            .await
            .expect("attach");
        let mut response = Box::pin(session.set_config_option(
            v1::SetSessionConfigOptionRequest::new(session.id().to_string(), "model", "deep"),
        ));
        assert!(futures::poll!(&mut response).is_pending());
        let response = if cancel_waiter {
            drop(response);
            None
        } else {
            Some(response)
        };
        tokio::time::timeout(TIMEOUT, async {
            for expected in [json!("deep"), json!(true)] {
                let event = events.recv().await.expect("ordered config update");
                let bas::model::EventKind::Acp(v1::SessionUpdate::ConfigOptionUpdate(update)) = event.kind else {
                    panic!("expected config update");
                };
                assert_eq!(serde_json::to_value(update.config_options).expect("options")[0]["currentValue"], expected);
            }
        }).await.expect("response then notification");
        if let Some(response) = response {
            response.await.expect("consume older response");
        }
        assert_eq!(config_value(&session), true);
        registry.shutdown().await;
        wait_for_exit(&directory).await;
    }
}
