use super::*;
use crate::config::{AgentSpec, Settings, Store, agent::Command};

fn agents() -> Vec<AgentRegistration> {
    vec![
        AgentSpec::embedded().into(),
        AgentSpec::acp("a", Command::new("a", [])).into(),
        AgentSpec::acp("b", Command::new("b", [])).into(),
    ]
}

#[test]
fn omissions_and_roundtrip_match_rust_defaults() {
    let defaults = AcpLifecycle::default();
    assert_eq!(
        defaults.idle_timeout_secs.expect("default enabled").get(),
        1800
    );
    assert_eq!(defaults.pool_size, 1);
    assert!(defaults.default_agent.is_none());
    assert!(defaults.pools.is_empty());
    for json in ["{}", r#"{"daemon":{}}"#, r#"{"daemon":{"acp":{}}}"#] {
        assert_eq!(
            serde_json::from_str::<Settings>(json).expect("settings"),
            Settings::default()
        );
    }
    let policy: AcpLifecycle = serde_json::from_str(
        r#"{"idle_timeout_secs":1800,"default_agent":"a","pool_size":1,"pools":{"b":2}}"#,
    )
    .expect("policy");
    assert_eq!(policy.idle_timeout_secs.expect("enabled").get(), 1800);
    assert_eq!(
        serde_json::from_slice::<AcpLifecycle>(&serde_json::to_vec(&policy).expect("serialize"))
            .expect("deserialize"),
        policy
    );
    for json in [
        r#"{"idle_timeout_secs":0}"#,
        r#"{"idle_timeout_secs":-1}"#,
        r#"{"pool_size":-1}"#,
        r#"{"unknown":1}"#,
    ] {
        assert!(
            serde_json::from_str::<AcpLifecycle>(json).is_err(),
            "{json}"
        );
    }
}

#[test]
fn partial_policy_keeps_defaults_and_explicit_disable_survives_roundtrip() {
    let partial: AcpLifecycle =
        serde_json::from_str(r#"{"default_agent":"a"}"#).expect("partial policy");
    assert_eq!(
        partial.idle_timeout_secs,
        AcpLifecycle::default().idle_timeout_secs
    );
    assert_eq!(
        partial.pool_targets(None),
        BTreeMap::from([("a".into(), 1)])
    );

    let disabled: AcpLifecycle =
        serde_json::from_str(r#"{"idle_timeout_secs":null,"pool_size":0}"#)
            .expect("explicit opt out");
    assert_eq!(disabled.idle_timeout_secs, None);
    assert_eq!(disabled.pool_size, 0);
    assert_eq!(
        disabled.pool_targets(Some(&"a".into())),
        BTreeMap::from([("a".into(), 0)])
    );
    assert_eq!(
        serde_json::from_slice::<AcpLifecycle>(&serde_json::to_vec(&disabled).expect("serialize"))
            .expect("deserialize"),
        disabled,
    );
}

#[test]
fn explicit_zero_wins_and_server_default_precedes_last_used() {
    let mut policy = AcpLifecycle {
        pool_size: 1,
        ..Default::default()
    };
    let a = AgentId::new("a");
    let b = AgentId::new("b");
    assert!(policy.pool_targets(None).is_empty());
    assert_eq!(
        policy.pool_targets(Some(&b)),
        BTreeMap::from([(b.clone(), 1)])
    );
    policy.default_agent = Some(a.clone());
    assert_eq!(
        policy.pool_targets(Some(&b)),
        BTreeMap::from([(a.clone(), 1)])
    );
    policy.pools.insert(a.clone(), 0);
    policy.pools.insert(b.clone(), 2);
    assert_eq!(
        policy.pool_targets(Some(&b)),
        BTreeMap::from([(a, 0), (b, 2)])
    );
}

#[test]
fn validates_ids_and_worst_case_reserve_without_overflow() {
    for id in ["", " ", "missing", "embedded"] {
        let policy = AcpLifecycle {
            default_agent: Some(id.into()),
            ..Default::default()
        };
        assert!(policy.validate(&agents()).is_err(), "{id}");
        let policy = AcpLifecycle {
            pools: BTreeMap::from([(id.into(), 0)]),
            ..Default::default()
        };
        assert!(policy.validate(&agents()).is_err(), "{id}");
    }
    let mut policy = AcpLifecycle {
        pool_size: 1,
        pools: BTreeMap::from([("a".into(), MAX_ACP_POOL_RESERVE)]),
        ..Default::default()
    };
    assert!(
        policy.validate(&agents()).is_err(),
        "last-used may add a reserve"
    );
    policy.default_agent = Some("a".into());
    assert!(policy.validate(&agents()).is_ok(), "explicit target wins");
    policy.pools.insert("b".into(), usize::MAX);
    assert!(policy.validate(&agents()).is_err(), "overflow is rejected");
}

#[test]
fn scoped_saves_preserve_lifecycle_and_missing_managed_installations() {
    let dir = tempfile::tempdir().expect("settings directory");
    let path = dir.path().join("settings.json");
    let mut settings = Settings::default();
    settings.daemon.agents = agents();
    settings.daemon.acp = AcpLifecycle {
        default_agent: Some("a".into()),
        pool_size: 1,
        ..Default::default()
    };
    settings
        .daemon
        .agents
        .push(AgentRegistration::Managed(crate::config::ManagedAgent {
            id: "managed".into(),
            installation: crate::config::InstallationRef {
                path: dir.path().join(crate::config::INSTALLATION_FILE),
                sha256: "0".repeat(64),
            },
        }));
    settings.daemon.acp.pools.insert("managed".into(), 1);
    std::fs::write(&path, serde_json::to_vec(&settings).expect("settings JSON")).expect("write");
    let store = Store::at(path);
    let mut document = store
        .load()
        .expect("missing installation does not block reading");
    store
        .save_appearance(&mut document, Default::default())
        .expect("appearance save");
    store
        .save_frontend(&mut document, Default::default())
        .expect("frontend save");
    store
        .save_agents(&mut document, settings.daemon.agents.clone())
        .expect("agents save");
    let original = std::fs::read(store.path()).expect("saved settings");
    assert!(
        store
            .save_agents(&mut document, vec![AgentSpec::embedded().into()])
            .is_err(),
        "removing policy targets must not save an invalid combined configuration"
    );
    assert_eq!(std::fs::read(store.path()).expect("unchanged"), original);
    assert_eq!(
        store.load().expect("reload").settings.daemon.acp,
        settings.daemon.acp
    );
}
