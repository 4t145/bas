use super::*;
use crate::service::Service;
use bas::service::{pb, pb::bas_server::Bas};
use tonic::Request;

fn spec(mode: &str, directory: &tempfile::TempDir) -> AgentSpec {
    let mut spec = AgentSpec::acp(
        "history",
        Command::new(
            "sh",
            [
                "-c".to_owned(),
                include_str!("../assets/history.sh").to_owned(),
            ],
        ),
    );
    spec.cancellation_grace = Duration::from_millis(200);
    spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
        directory.path().join("pid").to_str().expect("UTF-8").into(),
    );
    spec.env.insert(
        "BAS_FIXTURE_WORKSPACE".into(),
        directory.path().to_str().expect("UTF-8").into(),
    );
    spec
}

fn registry(mode: &str, directory: &tempfile::TempDir) -> Arc<Registry> {
    Arc::new(Registry::new(
        Arc::new(Catalog::from_specs([spec(mode, directory)]).expect("catalog")),
        Arc::new(LocalHost::default()),
        Limits {
            live_capacity: std::num::NonZeroUsize::new(4).expect("capacity"),
            ..Limits::default()
        },
    ))
}

fn import_request(directory: &tempfile::TempDir) -> CreateRequest {
    CreateRequest {
        agent: "history".into(),
        owner: Owner::current(),
        cwd: Some(directory.path().to_owned()),
        title: Some("External history".into()),
    }
}

fn recorded(directory: &tempfile::TempDir, method: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(directory.path().join(format!("pid.{method}")))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("fixture JSON"))
        .collect()
}

async fn events(session: &Arc<crate::session::Session>) -> Vec<bas::model::Event> {
    let mut events = Vec::new();
    session
        .try_visit_history(None, 100, |event, _| {
            events.push(event.clone());
            true
        })
        .await
        .expect("history");
    events
}

#[tokio::test]
async fn discovers_pages_on_one_connection_without_new_or_import() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("history", &directory);
    let service = Service::new(Arc::clone(&registry));
    let page = service
        .list_agent_sessions(Request::new(pb::ListAgentSessionsRequest {
            agent: "history".into(),
            cwd: None,
        }))
        .await
        .expect("discover")
        .into_inner();
    let sessions: v1::ListSessionsResponse =
        page.sessions.expect("sessions").decode().expect("ACP list");
    assert_eq!(sessions.sessions.len(), 1);
    assert!(sessions.next_cursor.is_none());
    assert_eq!(
        sessions.sessions[0].meta.as_ref().expect("meta")["origin"],
        "native-cli"
    );
    assert!(registry.list().await.is_empty());
    assert!(recorded(&directory, "new").is_empty());
    let requests = recorded(&directory, "list");
    assert_eq!(requests.len(), 2);
    assert!(requests[0]["params"].get("cursor").is_none());
    assert_eq!(requests[1]["params"]["cursor"], "opaque/cursor:2");
}

#[tokio::test]
async fn imports_replay_once_and_continues_using_the_original_agent_id() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("history", &directory);
    let (first, second) = tokio::join!(
        registry.import_agent_session(import_request(&directory), "fixture".into()),
        registry.import_agent_session(import_request(&directory), "fixture".into()),
    );
    let session = first.expect("first import");
    assert_eq!(second.expect("retry").id(), session.id());
    assert_eq!(registry.list().await.len(), 1);
    assert_eq!(recorded(&directory, "load").len(), 1);
    assert!(recorded(&directory, "new").is_empty());
    let replay = events(&session).await;
    assert_eq!(
        replay
            .iter()
            .filter(|event| matches!(
                event.kind,
                bas::model::EventKind::Acp(v1::SessionUpdate::AgentMessageChunk(_))
            ))
            .count(),
        12
    );
    assert_eq!(
        session.meta().acp.expect("ACP").session_id.to_string(),
        session.id().to_string()
    );
    let mut attachment = session
        .try_attach(AttachOptions {
            replay: 0,
            cursor: None,
        })
        .await
        .expect("attach");
    session
        .prompt(vec!["continue".into()])
        .await
        .expect("prompt");
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let event = attachment.recv().await.expect("event");
            if matches!(event.kind, bas::model::EventKind::TurnFinished { .. }) {
                break;
            }
        }
    })
    .await
    .expect("completion");
    assert_eq!(
        recorded(&directory, "prompt")[0]["params"]["sessionId"],
        "fixture"
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn standard_close_and_history_delete_retain_local_records() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("history", &directory);
    let service = Service::new(Arc::clone(&registry));
    let info = v1::SessionInfo::new("fixture", directory.path()).title("External history");
    let imported = service
        .import_agent_session(Request::new(pb::ImportAgentSessionRequest {
            agent: "history".into(),
            session_info: Some((&info).into()),
        }))
        .await
        .expect("RPC import")
        .into_inner()
        .session
        .expect("session");
    let public_id: SessionId = imported.id.parse().expect("UUID");
    assert!(
        service
            .delete_agent_session(Request::new(pb::DeleteAgentSessionRequest {
                agent: "history".into(),
                session_id: "fixture".into(),
            }))
            .await
            .is_err(),
        "must not delete history used by an open session"
    );
    assert!(recorded(&directory, "delete").is_empty());
    service
        .close_session(Request::new(pb::GetSessionRequest {
            session: imported.id.clone(),
        }))
        .await
        .expect("standard close");
    assert_eq!(recorded(&directory, "close").len(), 1);
    assert_eq!(
        recorded(&directory, "close")[0]["params"]["sessionId"],
        "fixture"
    );
    assert_eq!(
        registry
            .get(public_id)
            .await
            .expect("retained record")
            .meta()
            .status,
        SessionStatus::Stopped
    );
    service
        .delete_agent_session(Request::new(pb::DeleteAgentSessionRequest {
            agent: "history".into(),
            session_id: "fixture".into(),
        }))
        .await
        .expect("delete Agent history");
    assert!(
        registry.get(public_id).await.is_ok(),
        "Agent deletion is not local deletion"
    );
    assert_eq!(
        recorded(&directory, "delete")[0]["params"]["sessionId"],
        "fixture"
    );
    assert!(
        registry
            .list_agent_sessions("history".into(), None)
            .await
            .expect("rediscover")
            .sessions
            .sessions
            .is_empty()
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn unsupported_capabilities_are_checked_before_sending_methods() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("unsupported", &directory);
    assert!(
        registry
            .list_agent_sessions("history".into(), None)
            .await
            .is_err()
    );
    assert!(
        registry
            .delete_agent_session("history".into(), "fixture".into())
            .await
            .is_err()
    );
    assert!(recorded(&directory, "list").is_empty());
    assert!(recorded(&directory, "delete").is_empty());
    assert!(recorded(&directory, "new").is_empty());
}

#[tokio::test]
async fn repeating_pagination_and_failed_import_do_not_register_sessions() {
    for mode in ["repeat_cursor", "reject_load"] {
        let directory = tempfile::tempdir().expect("directory");
        let registry = registry(mode, &directory);
        if mode == "repeat_cursor" {
            assert!(
                registry
                    .list_agent_sessions("history".into(), None)
                    .await
                    .is_err()
            );
            assert_eq!(recorded(&directory, "list").len(), 2);
        } else {
            assert!(
                registry
                    .import_agent_session(import_request(&directory), "fixture".into())
                    .await
                    .is_err()
            );
        }
        assert!(registry.list().await.is_empty());
        assert!(recorded(&directory, "new").is_empty());
    }
}

#[tokio::test]
async fn close_rejection_keeps_the_session_and_native_error() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("reject_close", &directory);
    let session = registry
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("import");
    let service = Service::new(Arc::clone(&registry));
    let error = service
        .close_session(Request::new(pb::GetSessionRequest {
            session: session.id().to_string(),
        }))
        .await
        .expect_err("reject close");
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    let native: v1::Error = serde_json::from_slice(error.details()).expect("native error");
    assert_eq!(native.message, "closure rejected");
    assert_eq!(session.meta().status, SessionStatus::Idle);
    session
        .prompt(vec!["still usable".into()])
        .await
        .expect("can continue");
    registry.shutdown().await;
}

#[tokio::test]
async fn resume_only_import_is_explicitly_without_replayed_history() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("resume", &directory);
    let session = registry
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("resume");
    assert_eq!(recorded(&directory, "resume").len(), 1);
    assert!(recorded(&directory, "load").is_empty());
    let history = events(&session).await;
    assert!(!history.iter().any(|event| matches!(
        event.kind,
        bas::model::EventKind::Acp(v1::SessionUpdate::AgentMessageChunk(_))
    )));
    registry.shutdown().await;
}

#[tokio::test]
async fn imported_identity_and_transcript_survive_daemon_restart() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private directory");
    let database = directory.path().join("sessions.sqlite");
    let open = || async {
        Registry::with_storage(
            Arc::new(Catalog::from_specs([spec("history", &directory)]).expect("catalog")),
            Arc::new(LocalHost::default()),
            Limits::default(),
            Arc::new(
                crate::storage::LocalDb::open(&database)
                    .await
                    .expect("database"),
            ),
        )
    };
    let first = open().await;
    let session = first
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("import");
    let id = session.id();
    let before = events(&session).await;
    first.shutdown().await;
    drop(session);
    drop(first);

    let mut recovered = open().await;
    recovered.recover().await.expect("recover");
    assert_eq!(
        recovered.imported_sessions(&"history".into()).await["fixture"],
        id.to_string()
    );
    let session = recovered
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("repeat import");
    assert_eq!(session.id(), id);
    assert_eq!(
        recorded(&directory, "load").len(),
        1,
        "discovery does not activate the runtime"
    );
    recovered
        .ensure_restored(&session)
        .await
        .expect("restore context");
    assert_eq!(recorded(&directory, "load").len(), 2);
    assert_eq!(
        events(&session).await,
        before,
        "restore replay must not duplicate imported history"
    );
    assert!(recorded(&directory, "new").is_empty());
    recovered.shutdown().await;
}

#[tokio::test]
async fn local_deletion_does_not_delete_agent_history() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("history", &directory);
    let session = registry
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("import");
    registry.remove(session.id()).await.expect("local delete");
    assert_eq!(
        recorded(&directory, "close").len(),
        1,
        "release Agent resources before process cleanup"
    );
    assert!(recorded(&directory, "delete").is_empty());
    assert_eq!(
        registry
            .list_agent_sessions("history".into(), None)
            .await
            .expect("history")
            .sessions
            .sessions
            .len(),
        1
    );
}

#[tokio::test]
async fn native_delete_errors_and_discovery_deadlines_are_not_silently_successful() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("reject_delete", &directory);
    let service = Service::new(registry);
    let error = service
        .delete_agent_session(Request::new(pb::DeleteAgentSessionRequest {
            agent: "history".into(),
            session_id: "fixture".into(),
        }))
        .await
        .expect_err("reject delete");
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(!directory.path().join("pid.deleted").exists());

    let registry = Registry::new(
        Arc::new(Catalog::from_specs([spec("list_hang", &directory)]).expect("catalog")),
        Arc::new(LocalHost::default()),
        Limits {
            startup_timeout: Duration::from_millis(100),
            ..Limits::default()
        },
    );
    assert!(matches!(
        registry.list_agent_sessions("history".into(), None).await,
        Err(CreateError::Timeout)
    ));
    assert!(registry.list().await.is_empty());
}

#[tokio::test]
async fn timed_out_close_stops_the_uncertain_runtime_without_retrying() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("close_hang", &directory);
    let session = registry
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("import");
    assert!(
        session.close_backend().await.is_err(),
        "never report remote success after timeout"
    );
    assert_eq!(session.meta().status, SessionStatus::Stopped);
    assert_eq!(
        recorded(&directory, "close").len(),
        1,
        "cleanup must not retry an uncertain request"
    );
    assert!(
        session
            .prompt(vec!["must not dispatch".into()])
            .await
            .is_err()
    );
    assert!(recorded(&directory, "delete").is_empty());
    registry.shutdown().await;
}
