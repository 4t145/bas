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
fn composer_enter_preference_defaults_and_persists_with_existing_agent_configuration() {
    use appearance::ComposerEnterBehavior;

    let directory = tempfile::tempdir().expect("settings directory");
    let store = Store::at(directory.path().join("settings.json"));
    let mut document = store.load().expect("default settings");
    assert_eq!(
        document.settings.appearance.composer_enter_behavior,
        ComposerEnterBehavior::Send
    );
    let agents = document.settings.daemon.agents.clone();
    let appearance = Appearance {
        composer_enter_behavior: ComposerEnterBehavior::Newline,
        ..document.settings.appearance.clone()
    };
    store
        .save_appearance(&mut document, appearance)
        .expect("save input preference");
    let saved = store.load().expect("reload settings").settings;
    assert_eq!(
        saved.appearance.composer_enter_behavior,
        ComposerEnterBehavior::Newline
    );
    assert_eq!(saved.daemon.agents, agents);
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
