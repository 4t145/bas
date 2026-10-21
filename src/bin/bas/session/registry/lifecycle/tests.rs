use super::*;
use crate::agent::{AgentSpec, Command, LocalHost};
use crate::session::AttachOptions;
use bas::model::{EventKind, Owner};
use std::num::NonZeroU64;

mod restart;

#[cfg(unix)]
#[tokio::test]
async fn idle_runtime_reloads_original_session_without_breaking_attach() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let mut spec = AgentSpec::acp(
        "fixture",
        Command::new(
            "sh",
            ["-c", include_str!("../../../agent/acp/assets/fixture.sh")].map(str::to_owned),
        ),
    );
    spec.env.insert("BAS_FIXTURE_MODE".into(), "load".into());
    spec.env.insert(
        "BAS_FIXTURE_PID".into(),
        directory.path().join("pid").to_string_lossy().into_owned(),
    );
    let limits = Limits::default();
    let storage: Arc<dyn Storage> = Arc::new(storage::Memory::new(limits.history_capacity));
    let registry = Registry::with_storage(
        Arc::new(Catalog::from_specs([spec]).expect("catalog")),
        Arc::new(LocalHost::default()),
        limits,
        Arc::clone(&storage),
    );
    let policy = AcpLifecycle {
        idle_timeout_secs: NonZeroU64::new(1),
        pool_size: 0,
        ..Default::default()
    };
    registry.set_acp_policy(policy.clone()).expect("policy");
    let session = registry
        .create(CreateRequest {
            agent: "fixture".into(),
            owner: Owner::current(),
            cwd: None,
            title: None,
        })
        .await
        .expect("create");
    let id = session.id();
    let before = session.meta().revision;
    let mut attached = session
        .try_attach(AttachOptions {
            cursor: Some(before),
            replay: 0,
        })
        .await
        .expect("attach");
    let lease = registry.use_backend(&session).await.expect("pin runtime");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    registry.maintain_backends().await;
    assert!(
        session.is_restored(),
        "operation in flight prevents reclamation"
    );
    drop(lease);
    registry.maintain_backends().await;
    assert!(session.is_restored(), "release restarts idle interval");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    registry
        .set_acp_policy(AcpLifecycle {
            idle_timeout_secs: None,
            pool_size: 0,
            ..Default::default()
        })
        .expect("disable");
    registry.maintain_backends().await;
    assert!(session.is_restored(), "disabled policy keeps runtime");
    registry.set_acp_policy(policy).expect("enable");
    registry.maintain_backends().await;
    assert!(!session.is_restored());
    assert_eq!(session.meta().status, SessionStatus::Idle);
    assert_eq!(
        session.meta().revision,
        before,
        "reclaim is not a terminal event"
    );
    {
        let _lease = registry.use_backend(&session).await.expect("restore");
        assert!(session.is_restored());
        assert_eq!(session.id(), id);
        let load: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("pid.load")).expect("load request"),
        )
        .expect("load JSON");
        assert_eq!(load["params"]["sessionId"], "fixture");
        session.set_title("after reload").await.expect("live title");
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(attached.recv().await.expect("live subscription").kind,
                EventKind::TitleChanged { title } if title == "after reload")
            {
                break;
            }
        }
    })
    .await
    .expect("existing subscriber receives restored runtime events");
    let page = storage.page(id, None, 100).await.expect("history");
    assert!(
        !serde_json::to_string(&page.events)
            .expect("events")
            .contains("replayed"),
        "load replay must not duplicate durable history"
    );
    registry.shutdown().await;
}
