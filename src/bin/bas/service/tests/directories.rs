use super::*;
use crate::{
    agent::{AgentSpec, Command},
    storage::{LocalDb, Storage},
};
use bas::service::Client;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::task::AbortOnDropHandle;

struct Fixture {
    directory: tempfile::TempDir,
    registry: Arc<Registry>,
    session: Arc<crate::session::Session>,
    client: Client,
    catalog: Arc<Catalog>,
    storage: Arc<dyn Storage>,
    _server: AbortOnDropHandle<Result<(), tonic::transport::Error>>,
}

impl Fixture {
    async fn new(mode: &str) -> Self {
        let directory = tempfile::tempdir().expect("directory");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private database directory");
        let mut spec = AgentSpec::acp(
            "roots",
            Command::new(
                "sh",
                [
                    "-c".into(),
                    include_str!("../../agent/acp/assets/directories.sh").into(),
                ],
            ),
        );
        spec.env.insert("BAS_FIXTURE_MODE".into(), mode.into());
        spec.env.insert(
            "BAS_FIXTURE_REQUESTS".into(),
            directory
                .path()
                .join("requests")
                .to_str()
                .expect("UTF-8")
                .into(),
        );
        let catalog = Arc::new(Catalog::from_specs([spec]).expect("catalog"));
        let storage: Arc<dyn Storage> = Arc::new(
            LocalDb::open(directory.path().join("sessions.sqlite"))
                .await
                .expect("database"),
        );
        let registry = Arc::new(Registry::with_storage(
            catalog.clone(),
            Arc::new(LocalHost::default()),
            Limits {
                startup_timeout: Duration::from_secs(2),
                ..Limits::default()
            },
            storage.clone(),
        ));
        let session = registry
            .create(CreateRequest {
                agent: "roots".into(),
                owner: Owner::current(),
                cwd: Some(directory.path().into()),
                title: None,
            })
            .await
            .expect("session");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let server = AbortOnDropHandle::new(tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(Service::new(registry.clone()).into_server())
                .serve_with_incoming(TcpListenerStream::new(listener)),
        ));
        let client = Client::connect(format!("http://{address}"))
            .await
            .expect("client");
        Self {
            directory,
            registry,
            session,
            client,
            catalog,
            storage,
            _server: server,
        }
    }

    fn request(&self, paths: Vec<String>) -> pb::SetSessionDirectoriesRequest {
        pb::SetSessionDirectoriesRequest {
            session: self.session.id().to_string(),
            additional_directories: paths,
        }
    }

    fn recorded(&self, method: &str) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.directory.path().join("requests"))
            .expect("request log")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSON"))
            .filter(|request| request["method"] == method)
            .collect()
    }
}

#[tokio::test]
async fn additional_directories_reload_same_session_and_survive_restart_without_duplicate_history()
{
    for mode in ["load", "resume"] {
        let mut fixture = Fixture::new(mode).await;
        let root = tempfile::tempdir().expect("extra root");
        let path = root.path().to_str().expect("UTF-8").to_owned();
        let before = fixture.session.meta();
        let snapshot: bas::model::SessionSnapshot = fixture
            .client
            .set_session_directories(fixture.request(vec![path.clone(), path.clone()]))
            .await
            .expect("reload")
            .into_inner()
            .session
            .expect("snapshot")
            .try_into()
            .expect("decode");
        assert_eq!(snapshot.meta.id, before.id);
        assert_eq!(snapshot.meta.cwd, before.cwd);
        assert_eq!(snapshot.meta.additional_directories, [root.path()]);
        assert_eq!(fixture.recorded("session/new").len(), 1);
        let method = if mode == "load" {
            "session/load"
        } else {
            "session/resume"
        };
        let requests = fixture.recorded(method);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["params"]["sessionId"], "same-native-id");
        assert_eq!(
            requests[0]["params"]["cwd"],
            before.cwd.as_ref().expect("cwd").to_str().expect("UTF-8")
        );
        assert_eq!(
            requests[0]["params"]["additionalDirectories"],
            serde_json::json!([path])
        );
        assert_eq!(
            fixture
                .storage
                .load(before.id)
                .await
                .expect("store")
                .expect("record")
                .meta
                .additional_directories,
            [root.path()]
        );
        fixture.session.close_backend().await.expect("close");
        fixture
            .registry
            .restart_session(&fixture.session)
            .await
            .expect("restart");
        assert_eq!(
            fixture.recorded(method)[1]["params"]["additionalDirectories"],
            serde_json::json!([path])
        );
        let mut history = Vec::new();
        fixture
            .session
            .try_visit_history(None, 100, |event, _| {
                history.push(event.clone());
                true
            })
            .await
            .expect("history");
        assert!(!history.iter().any(|event| matches!(
            event.kind,
            bas::model::EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(_))
        )));
        assert!(
            history
                .iter()
                .any(|event| matches!(event.kind, bas::model::EventKind::WorkspaceChanged { .. }))
        );
        // A cold daemon recovery uses the persisted roots too.
        fixture.registry.shutdown().await;
        let reopened: Arc<dyn Storage> = Arc::new(
            LocalDb::open(fixture.directory.path().join("sessions.sqlite"))
                .await
                .expect("reopen database"),
        );
        let mut recovered = Registry::with_storage(
            fixture.catalog.clone(),
            Arc::new(LocalHost::default()),
            Limits::default(),
            reopened,
        );
        recovered.recover().await.expect("recover");
        let session = recovered.get(before.id).await.expect("recovered session");
        recovered.ensure_restored(&session).await.expect("restore");
        assert_eq!(
            fixture.recorded(method).last().expect("last load")["params"]["additionalDirectories"],
            serde_json::json!([path])
        );
        session
            .set_directories(Vec::new(), Duration::from_secs(2))
            .await
            .expect("clear roots");
        assert!(session.meta().additional_directories.is_empty());
        assert!(
            fixture.recorded(method).last().expect("clear load")["params"]
                .get("additionalDirectories")
                .is_none()
        );
        recovered.shutdown().await;
    }
}

#[tokio::test]
async fn active_turns_refuse_reload_and_timeout_stops_uncertain_runtime() {
    for mode in ["working", "hang"] {
        let fixture = Fixture::new(mode).await;
        let root = tempfile::tempdir().expect("root");
        if mode == "working" {
            fixture
                .session
                .prompt(vec!["work".into()])
                .await
                .expect("start turn");
        }
        fixture
            .session
            .set_directories(vec![root.path().into()], Duration::from_millis(100))
            .await
            .expect_err("busy or timeout");
        assert!(fixture.session.meta().additional_directories.is_empty());
        if mode == "working" {
            assert!(fixture.recorded("session/load").is_empty());
        } else {
            assert_eq!(
                fixture.session.meta().status,
                bas::model::SessionStatus::Failed
            );
            assert!(
                fixture
                    .session
                    .prompt(vec!["do not use uncertain scope".into()])
                    .await
                    .is_err()
            );
        }
        fixture.registry.shutdown().await;
    }
}
