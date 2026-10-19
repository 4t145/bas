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
async fn restore_starts_the_archived_session_without_duplicating_history() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("history", &directory);
    let session = registry
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("import");
    let archived = registry
        .set_archived(&session, true)
        .await
        .expect("archive");
    assert_eq!(archived.meta.status, SessionStatus::Stopped);
    assert!(archived.meta.archived);
    assert_eq!(recorded(&directory, "close").len(), 1);
    let previous = events(&session).await;
    let service = Service::new(Arc::clone(&registry));
    assert!(
        service
            .restart_session(Request::new(pb::GetSessionRequest {
                session: session.id().to_string(),
            }))
            .await
            .is_err(),
        "archived sessions must be restored, not directly started"
    );
    for mode in ["reject_load", "unsupported"] {
        registry.replace_catalog(Arc::new(
            Catalog::from_specs([spec(mode, &directory)]).expect("catalog"),
        ));
        assert!(registry.set_archived(&session, false).await.is_err());
        assert_eq!(session.snapshot(), archived);
        assert_eq!(events(&session).await.len(), previous.len());
    }
    registry.replace_catalog(Arc::new(
        Catalog::from_specs([spec("history", &directory)]).expect("catalog"),
    ));
    let loads = recorded(&directory, "load").len();
    let response = service
        .set_archived(Request::new(pb::SetArchivedRequest {
            session: session.id().to_string(),
            archived: false,
        }))
        .await
        .expect("restore RPC")
        .into_inner()
        .session
        .expect("snapshot");
    let snapshot: bas::model::SessionSnapshot = response.try_into().expect("typed snapshot");
    assert_eq!(snapshot.meta.id, session.id());
    assert_eq!(snapshot.meta.status, SessionStatus::Idle);
    assert!(
        !snapshot.meta.archived,
        "restore unarchives and starts together"
    );
    assert_eq!(registry.list().await.len(), 1);
    let after = events(&session).await;
    assert_eq!(
        after.len(),
        previous.len() + 2,
        "only restore and ready transitions are added"
    );
    assert_eq!(recorded(&directory, "load").len(), loads + 1);
    assert!(recorded(&directory, "new").is_empty());
    let mut attachment = session
        .try_attach(AttachOptions {
            replay: 0,
            cursor: None,
        })
        .await
        .expect("new live stream");
    session
        .prompt(vec!["continue restarted session".into()])
        .await
        .expect("prompt");
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
    .expect("restart completes a turn");
    assert_eq!(
        recorded(&directory, "prompt")[0]["params"]["sessionId"],
        "fixture"
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn cancelled_restart_does_not_change_or_lock_the_stopped_record() {
    let directory = tempfile::tempdir().expect("directory");
    let registry = registry("history", &directory);
    let session = registry
        .import_agent_session(import_request(&directory), "fixture".into())
        .await
        .expect("import");
    session.close_backend().await.expect("close");
    let before = session.snapshot();
    registry.replace_catalog(Arc::new(
        Catalog::from_specs([spec("load_hang", &directory)]).expect("catalog"),
    ));
    let running_registry = registry.clone();
    let running_session = session.clone();
    let task =
        tokio::spawn(async move { running_registry.restart_session(&running_session).await });
    tokio::time::timeout(TIMEOUT, async {
        while recorded(&directory, "load").len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("staged load");
    task.abort();
    assert!(task.await.expect_err("cancelled task").is_cancelled());
    assert_eq!(session.snapshot(), before);
    registry.replace_catalog(Arc::new(
        Catalog::from_specs([spec("resume", &directory)]).expect("catalog"),
    ));
    registry
        .restart_session(&session)
        .await
        .expect("retry with resume-only Agent");
    assert_eq!(session.meta().status, SessionStatus::Idle);
    assert_eq!(recorded(&directory, "resume").len(), 1);
    assert!(recorded(&directory, "new").is_empty());
    registry.shutdown().await;
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
    session.close_backend().await.expect("durable close");
    session.set_archived(true).await;
    let stopped_history = events(&session).await;
    recovered.shutdown().await;
    drop(session);
    drop(recovered);
    let mut reopened = open().await;
    reopened.recover().await.expect("recover stopped record");
    let session = reopened.get(id).await.expect("same stopped ID");
    assert_eq!(session.meta().status, SessionStatus::Stopped);
    reopened
        .set_archived(&session, false)
        .await
        .expect("restart after daemon reboot");
    assert_eq!(session.meta().status, SessionStatus::Idle);
    assert!(!session.meta().archived);
    assert_eq!(recorded(&directory, "load").len(), 3);
    assert_eq!(events(&session).await.len(), stopped_history.len() + 2);
    reopened.shutdown().await;
}
