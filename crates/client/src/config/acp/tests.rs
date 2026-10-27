use super::*;
use crate::config::{AgentSpec, Daemon, agent::Command};

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
    for json in ["{}", r#"{"acp":{}}"#] {
        assert_eq!(
            serde_json::from_str::<Daemon>(json).expect("settings"),
            Daemon::default()
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
