use super::*;
use crate::{
    agent::{AgentSpec, Command},
    storage::{LocalDb, Storage},
};
use bas::model::{EventKind, SessionSnapshot, SessionStatus};

#[tokio::test]
async fn subagents_flow_through_snapshots_history_and_reload_without_becoming_parent_messages() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("fixture");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private directory");
    let file = directory.path().join("child.txt");
    std::fs::write(&file, "child file result").expect("child file");
    let requests = directory.path().join("requests");
    let mut spec = AgentSpec::acp(
        "subagents",
        Command::new(
            "sh",
            vec![
                "-c".into(),
                include_str!("../../agent/acp/assets/subagents.sh").into(),
            ],
        ),
    );
    spec.env.insert(
        "BAS_FIXTURE_FILE".into(),
        file.to_str().expect("UTF-8").into(),
    );
    spec.env.insert(
        "BAS_FIXTURE_REQUESTS".into(),
        requests.to_str().expect("UTF-8").into(),
    );
    let catalog = Arc::new(Catalog::from_specs([spec]).expect("catalog"));
    let database = directory.path().join("sessions.sqlite");
    let storage: Arc<dyn Storage> = Arc::new(LocalDb::open(&database).await.expect("database"));
    let registry = Arc::new(Registry::with_storage(
        catalog.clone(),
        Arc::new(LocalHost::default()),
        Limits::default(),
        storage.clone(),
    ));
    let session = registry
        .create(CreateRequest {
            agent: "subagents".into(),
            owner: Owner::current(),
            cwd: Some(directory.path().into()),
            title: None,
        })
        .await
        .expect("create with subagents capability");
    let id = session.id();
    session
        .prompt(vec!["delegate".into()])
        .await
        .expect("prompt");
    tokio::time::timeout(Duration::from_secs(3), async {
        while session.meta().status != SessionStatus::Idle
            || session
                .meta()
                .subagents
                .iter()
                .filter(|s| s.is_active())
                .count()
                != 2
        {
            tokio::task::yield_now().await;
        }
        while {
            let responses = std::fs::read_to_string(&requests).expect("requests");
            !responses.contains("child file result") || !responses.contains("foreign-read")
        } {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("child states after parent completes");
    assert_eq!(session.meta().last_reply, Cursor::START);
    let service = Service::new(registry.clone());
    let snapshot: SessionSnapshot = service
        .get_session(Request::new(pb::GetSessionRequest {
            session: id.to_string(),
        }))
        .await
        .expect("snapshot")
        .into_inner()
        .session
        .expect("session")
        .try_into()
        .expect("decode");
    assert_eq!(
        snapshot
            .meta
            .subagents
            .iter()
            .filter(|child| child.is_active())
            .count(),
        2
    );
    let nested = snapshot
        .meta
        .subagents
        .iter()
        .find(|child| child.id == "nested")
        .expect("nested");
    assert_eq!(nested.parent.as_deref(), Some("child"));
    let responses: Vec<serde_json::Value> = std::fs::read_to_string(&requests)
        .expect("requests")
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON"))
        .collect();
    assert!(
        responses
            .iter()
            .any(|r| r["id"] == "foreign-read" && r.get("error").is_some())
    );
    let history = service
        .load_history(Request::new(pb::LoadHistoryRequest {
            session: id.to_string(),
            after: None,
            limit: 100,
        }))
        .await
        .expect("history")
        .into_inner()
        .events;
    let events: Vec<bas::model::Event> = history
        .into_iter()
        .map(|event| event.try_into().expect("event"))
        .collect();
    assert!(events.iter().any(
        |event| matches!(&event.kind, EventKind::Subagent(n) if n.session_id.0.as_ref() == "nested")
    ));
    assert!(!events.iter().any(|event| matches!(
        event.kind,
        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(_))
    )));
    assert_eq!(
        storage
            .load(id)
            .await
            .expect("stored")
            .expect("session")
            .meta
            .subagents,
        snapshot.meta.subagents
    );
    // A new daemon must not claim old child activity is still live.
    registry.shutdown().await;
    let reopened: Arc<dyn Storage> = Arc::new(LocalDb::open(&database).await.expect("reopen"));
    let mut recovered = Registry::with_storage(
        catalog,
        Arc::new(LocalHost::default()),
        Limits::default(),
        reopened,
    );
    recovered.recover().await.expect("recover");
    let restored = recovered.get(id).await.expect("restored");
    assert!(
        !restored
            .meta()
            .subagents
            .iter()
            .any(|child| child.is_active())
    );
    recovered
        .ensure_restored(&restored)
        .await
        .expect("load original session");
    assert_eq!(restored.meta().subagents.iter().count(), 2);
    assert!(
        !restored
            .meta()
            .subagents
            .iter()
            .any(|child| child.is_active())
    );
    restored
        .prompt(vec!["delegate again".into()])
        .await
        .expect("reuse children");
    tokio::time::timeout(Duration::from_secs(3), async {
        while restored.meta().status != SessionStatus::Idle
            || restored
                .meta()
                .subagents
                .iter()
                .filter(|s| s.is_active())
                .count()
                != 2
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reused children run");
    restored
        .set_directories(vec![std::env::temp_dir()], Duration::from_secs(3))
        .await
        .expect("reload scope");
    assert!(
        !restored
            .meta()
            .subagents
            .iter()
            .any(|child| child.is_active())
    );
    recovered.shutdown().await;
}
