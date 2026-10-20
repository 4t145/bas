use super::*;
use crate::config::{Appearance, Error, Settings, Store};
use std::fs;

#[test]
fn mtls_profiles_store_only_references_and_separate_identity_state() {
    let value = serde_json::json!({
        "id": "secure", "name": "Secure", "target": "mtls://workstation:7338",
        "tls": {
            "ca": "/certs/ca.pem", "certificate": "/certs/client.pem",
            "private_key": "/certs/client.key", "server_name": "workstation"
        }
    });
    let profile: ConnectionProfile =
        serde_json::from_value(value.clone()).expect("file references");
    assert_eq!(serde_json::to_value(&profile).expect("encode"), value);
    let first = profile.options();
    first
        .validate()
        .expect("no files read during settings validation");
    assert!(!first.is_local());
    assert!(!first.is_loopback_tcp());
    let mut second = first.clone();
    second.tls.as_mut().expect("TLS").private_key = "/certs/other-client.key".into();
    assert_ne!(first, second);
    assert_ne!(
        first.state_key().expect("first key"),
        second.state_key().expect("second key")
    );
    second.tls = None;
    assert!(second.validate().is_err());
    second.tls = first.tls.clone();
    second.target = Target::Local;
    assert!(second.validate().is_err());
    second.target = first.target.clone();
    second.tls.as_mut().expect("TLS").ca = "".into();
    assert!(second.validate().is_err());
}

#[test]
fn defaults_and_old_documents_are_read_without_writing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::at(dir.path().join("settings.json"));
    let frontend = store.load().expect("defaults").settings.frontend;
    assert_eq!(frontend.selected, "local");
    assert_eq!(
        frontend.selected_connection(),
        Some(&ConnectionProfile {
            id: "local".into(),
            name: "Local".into(),
            note: None,
            target: Target::Local,
            tls: None,
        })
    );
    assert!(!store.path().exists());
    let old = r#"{"appearance":{},"daemon":{}}"#;
    fs::write(store.path(), old).expect("old settings");
    assert_eq!(store.load().expect("migration").settings.frontend, frontend);
    assert_eq!(fs::read_to_string(store.path()).expect("unchanged"), old);
}

#[test]
fn target_strings_roundtrip_without_platform_restrictions() {
    for syntax in [
        "local",
        "unix:///tmp/bas.sock",
        "pipe://bas-test",
        "tcp://127.0.0.1:50051",
        "127.0.0.1:50051",
        "tcp://[::1]:50051",
        "[::1]:50051",
        "tcp://workstation",
        "workstation.tailnet.ts.net",
        "not-resolvable.invalid",
    ] {
        let value = serde_json::json!({"id":"test","name":"Test","target":syntax});
        let profile: ConnectionProfile = serde_json::from_value(value).expect("target");
        let encoded = serde_json::to_value(&profile).expect("encode");
        assert_eq!(profile.note, None);
        assert!(encoded.get("note").is_none());
        assert_eq!(encoded["target"], profile.target.to_string());
        assert_eq!(
            serde_json::from_value::<ConnectionProfile>(encoded).expect("decode"),
            profile
        );
        let frontend = Frontend {
            connections: vec![profile],
            selected: "test".into(),
        };
        frontend
            .validate()
            .expect("platform-independent validation");
    }
}

#[test]
fn optional_notes_roundtrip_without_affecting_connection_identity() {
    let mut profile = Frontend::default().connections.remove(0);
    let original_target = profile.target.clone();
    profile.note = Some("用于日常开发\n连接备用工作站".into());
    let encoded = serde_json::to_value(&profile).expect("encode note");
    let decoded: ConnectionProfile = serde_json::from_value(encoded).expect("decode note");
    assert_eq!(decoded, profile);
    assert_eq!(decoded.id, "local");
    assert_eq!(decoded.target, original_target);
}

#[test]
fn strict_syntax_and_domain_validation() {
    for text in [
        r#"{"frontend":{"unknown":1}}"#,
        r#"{"frontend":{"connections":[{"id":"x","name":"X","target":"local","unknown":1}]}}"#,
        r#"{"frontend":{"connections":[{"id":"x","name":"X","target":"unix://relative"}]}}"#,
        r#"{"frontend":{"connections":[{"id":"x","name":"X","target":"pipe://"}]}}"#,
        r#"{"frontend":{"connections":[{"id":"x","name":"X","target":"tcp://host:65536"}]}}"#,
        r#"{"frontend":{"connections":[{"id":"x","name":"X","target":{}}]}}"#,
    ] {
        assert!(serde_json::from_str::<Settings>(text).is_err(), "{text}");
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::at(dir.path().join("settings.json"));
    for mutate in [
        |f: &mut Frontend| f.connections.clear(),
        |f: &mut Frontend| f.connections[0].id = " \t".into(),
        |f: &mut Frontend| f.connections[0].name = " \n".into(),
        |f: &mut Frontend| f.connections.push(f.connections[0].clone()),
        |f: &mut Frontend| f.selected = "missing".into(),
    ] {
        let mut frontend = Frontend::default();
        mutate(&mut frontend);
        assert!(frontend.validate().is_err());
        let settings = Settings {
            frontend,
            ..Settings::default()
        };
        fs::write(store.path(), serde_json::to_vec(&settings).expect("encode")).expect("write");
        assert!(matches!(store.load(), Err(Error::Invalid { .. })));
    }
    let mut frontend = Frontend::default();
    let mut duplicate_target = frontend.connections[0].clone();
    duplicate_target.id = "other".into();
    frontend.connections.push(duplicate_target);
    frontend.validate().expect("duplicate addresses allowed");
}

#[test]
fn section_saves_merge_and_frontend_conflicts_preserve_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::at(dir.path().join("settings.json"));
    let mut frontend_document = store.load().expect("load");
    let mut stale = frontend_document.clone();
    let mut external = frontend_document.clone();
    let appearance = Appearance {
        ui_font_size: Some(19.),
        ..Appearance::default()
    };
    store
        .save_appearance(&mut external, appearance.clone())
        .expect("external appearance");
    let mut spec = crate::config::AgentSpec::embedded();
    spec.id = "renamed".into();
    let agents = vec![spec.into()];
    store
        .save_agents(&mut external, agents.clone())
        .expect("external agents");
    let mut frontend = Frontend::default();
    frontend.connections[0].name = "My local service".into();
    store
        .save_frontend(&mut frontend_document, frontend.clone())
        .expect("merge");
    assert_eq!(frontend_document.settings.appearance, appearance);
    assert_eq!(frontend_document.settings.daemon.agents, agents);
    let saved = fs::read(store.path()).expect("saved bytes");
    assert!(matches!(
        store.save_frontend(&mut stale, Frontend::default()),
        Err(Error::Conflict {
            section: "frontend",
            ..
        })
    ));
    assert_eq!(fs::read(store.path()).expect("unchanged"), saved);
    store
        .save_appearance(&mut external, Appearance::default())
        .expect("preserve frontend");
    store
        .save_agents(&mut external, Settings::default().daemon.agents)
        .expect("preserve frontend");
    assert_eq!(store.load().expect("load").settings.frontend, frontend);
}
