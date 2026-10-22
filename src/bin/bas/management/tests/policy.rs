use super::*;
use crate::{
    agent::{Catalog as AgentCatalog, LocalHost},
    session::{Limits, Registry},
};
use bas::{config::AcpLifecycle, service::pb};

#[tokio::test]
async fn policy_edit_preserves_agents_applies_immediately_and_rejects_stale_clients() {
    let directory = tempfile::tempdir().expect("isolated policy");
    let manager = fixture::manager(&directory, "http://127.0.0.1:1");
    let store = manager.store().expect("store");
    let mut document = store.load().expect("document");
    let agents = vec![AgentSpec::acp("a", Command::new("unused", [])).into()];
    store
        .save_agents(&mut document, agents.clone())
        .expect("agents");
    let registry = Registry::new(
        Arc::new(AgentCatalog::from_registrations(agents.clone()).expect("catalog")),
        Arc::new(LocalHost::default()),
        Limits::default(),
    );
    let baseline = manager.acp_configuration().expect("configuration");
    let policy = AcpLifecycle {
        idle_timeout_secs: None,
        default_agent: Some("a".into()),
        pool_size: 2,
        pools: BTreeMap::from([("a".into(), 0)]),
    };
    let request = pb::SaveAcpConfigurationRequest {
        profile: baseline.profile,
        revision: baseline.revision,
        policy_json: serde_json::to_vec(&policy).expect("JSON"),
    };
    let saved = manager
        .save_acp_configuration(request.clone(), &registry)
        .await
        .expect("save");
    assert_eq!(registry.acp_policy(), policy);
    assert_eq!(store.load().expect("disk").settings.agents, agents);
    assert_eq!(store.load().expect("disk").settings.acp, policy);
    assert_eq!(
        manager
            .save_acp_configuration(request.clone(), &registry)
            .await
            .expect_err("stale")
            .code(),
        tonic::Code::Aborted
    );
    let invalid = pb::SaveAcpConfigurationRequest {
        profile: saved.profile.clone(),
        revision: saved.revision.clone(),
        policy_json: br#"{"default_agent":"missing"}"#.to_vec(),
    };
    assert_eq!(
        manager
            .save_acp_configuration(invalid, &registry)
            .await
            .expect_err("unknown ID")
            .code(),
        tonic::Code::InvalidArgument
    );
    assert_eq!(registry.acp_policy(), policy);
    let foreign = pb::SaveAcpConfigurationRequest {
        profile: "another server".into(),
        revision: saved.revision,
        policy_json: request.policy_json,
    };
    assert_eq!(
        manager
            .save_acp_configuration(foreign, &registry)
            .await
            .expect_err("wrong server")
            .code(),
        tonic::Code::FailedPrecondition
    );
}
