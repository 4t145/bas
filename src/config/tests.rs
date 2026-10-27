use super::*;

#[test]
fn strict_json_and_domain_rules() {
    for text in [
        r#"{"unknown":1}"#,
        r#"{"appearance":{"unknown":1}}"#,
        r#"{"daemon":{"unknown":1}}"#,
        r#"{"daemon":{"agents":[{"id":"x","kind":"embedded","unknown":1}]}}"#,
        r#"{"daemon":{"agents":[{"id":"x","kind":"acp","command":{"program":"dummy","unknown":1}}]}}"#,
    ] {
        assert!(serde_json::from_str::<Settings>(text).is_err(), "{text}");
    }
    assert_eq!(
        serde_json::from_str::<Settings>("{}").expect("defaults"),
        Settings::default()
    );
    for millis in ["-1", "0.5", "18446744073709551616"] {
        assert!(
            serde_json::from_str::<AgentSpec>(&format!(
                r#"{{"id":"x","kind":"embedded","cancellation_grace_ms":{millis}}}"#
            ))
            .is_err()
        );
    }
    assert!(validate_agents(&[]).is_err());
    assert!(validate_agents(&[AgentSpec::embedded(), AgentSpec::embedded()]).is_err());
    for text in [
        r#"{"id":" ","kind":"embedded"}"#,
        r#"{"id":"x","kind":"embedded","command":{"program":"dummy"}}"#,
        r#"{"id":"x","kind":"acp"}"#,
        r#"{"id":"x","kind":"acp","command":{"program":""}}"#,
        r#"{"id":"x","kind":"embedded","cancellation_grace_ms":0}"#,
    ] {
        assert!(validate_agents(&[serde_json::from_str(text).expect("structural JSON")]).is_err());
    }
    for value in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(
            Appearance {
                ui_font_size: Some(value),
                ..Appearance::default()
            }
            .validate()
            .is_err()
        );
    }
    for text in [
        r#"{"content_opacity":2}"#,
        r#"{"font_fallbacks":[" "]}"#,
        r#"{"theme_file":""}"#,
        r#"{"composer_enter_behavior":"unsupported"}"#,
    ] {
        assert!(Appearance::from_json(text).is_err());
    }
    assert_eq!(
        " BUILTIN ".parse::<agent::Kind>().expect("alias"),
        agent::Kind::Embedded
    );
    assert!("x=embedded:dummy".parse::<AgentSpec>().is_err());
    assert!("x=acp".parse::<AgentSpec>().is_err());
    assert!("=embedded".parse::<AgentSpec>().is_err());
}

#[test]
fn sections_merge_and_conflict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::at(dir.path().join("settings.json"));
    let mut a = store.load().expect("load");
    let mut b = a.clone();
    let appearance = Appearance {
        ui_font_size: Some(19.),
        ..Appearance::default()
    };
    store
        .save_appearance(&mut a, appearance.clone())
        .expect("appearance save");
    store
        .save_agents(
            &mut b,
            vec![
                AgentSpec::acp(
                    "dummy",
                    agent::Command::new("dummy", ["arg with spaces".into()]),
                )
                .into(),
            ],
        )
        .expect("agents save");
    assert_eq!(store.load().expect("load").settings.appearance, appearance);
    assert!(matches!(
        store.save_appearance(&mut b, Appearance::default()),
        Ok(())
    ));
    assert!(matches!(
        store.save_appearance(&mut a, appearance),
        Err(Error::Conflict { .. })
    ));
}

#[test]
fn daemon_validation_does_not_resolve_managed_installations() {
    let directory = tempfile::tempdir().expect("installation parent");
    let path = directory.path().join(INSTALLATION_FILE);
    let daemon = Daemon {
        agents: vec![AgentRegistration::Managed(ManagedAgent {
            id: "managed".into(),
            installation: InstallationRef::from_bytes(path.clone(), b"not read by validation")
                .expect("valid reference shape"),
        })],
        acp: AcpLifecycle {
            default_agent: Some("managed".into()),
            ..AcpLifecycle::default()
        },
    };
    daemon
        .validate()
        .expect("missing installation does not block settings validation");
    assert!(!path.exists());
    assert!(
        daemon.agents[0].resolve().is_err(),
        "resolution still requires the manifest"
    );
}

#[test]
fn scoped_saves_preserve_lifecycle_and_missing_managed_installations() {
    let dir = tempfile::tempdir().expect("settings directory");
    let path = dir.path().join("settings.json");
    let mut settings = Settings::default();
    settings.daemon.agents = vec![
        AgentSpec::embedded().into(),
        AgentSpec::acp("a", agent::Command::new("a", [])).into(),
        AgentSpec::acp("b", agent::Command::new("b", [])).into(),
    ];
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
