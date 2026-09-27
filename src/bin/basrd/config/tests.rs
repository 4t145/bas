use std::str::FromStr;

use clap::Parser;

use crate::agent::Kind;

use super::{Notify, Storage, parse_agents};

#[test]
fn parses_the_embedded_agent() {
    let agents = parse_agents("embedded").unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].kind, Kind::Embedded);
    assert_eq!(agents[0].id.as_str(), "embedded");
    assert!(agents[0].command().is_none());
}

#[test]
fn parses_acp_agents_with_arguments() {
    let agents = parse_agents(" embedded ; opencode=acp:opencode acp --print-logs ").unwrap();

    assert_eq!(agents.len(), 2);
    let acp = &agents[1];
    assert_eq!(acp.kind, Kind::Acp);
    assert_eq!(acp.id.as_str(), "opencode");
    let command = acp.command().unwrap();
    assert_eq!(command.program.to_string_lossy(), "opencode");
    assert_eq!(command.args, vec!["acp", "--print-logs"]);
}

#[test]
fn rejects_an_acp_agent_without_a_command() {
    let error = parse_agents("opencode=acp").unwrap_err();

    assert!(error.to_string().contains("has no command line"));
}

#[test]
fn rejects_an_unknown_kind() {
    let error = parse_agents("thing=telepathy").unwrap_err();

    assert!(error.to_string().contains("unknown agent kind"));
}

#[test]
fn parses_notification_policies() {
    assert_eq!(Notify::from_str(" AUTO ").unwrap(), Notify::Auto);
    assert_eq!(Notify::from_str("always").unwrap(), Notify::Always);
    assert_eq!(Notify::from_str("off").unwrap(), Notify::Never);
    assert!(Notify::from_str("sometimes").is_err());
}

#[test]
fn defaults_are_suitable_for_a_local_daemon() {
    let config = super::Config::default();

    assert!(config.listen.ip().is_loopback());
    assert_eq!(config.notify, Notify::Auto);
    assert_eq!(config.storage, Storage::Localdb);
    assert_eq!(config.localdb_path, None);
    assert_eq!(config.agents, vec![crate::agent::AgentSpec::embedded()]);
}

#[test]
fn memory_backend_remains_an_explicit_opt_out() {
    let config = super::Config {
        storage: Storage::Memory,
        ..super::Config::default()
    };
    assert_eq!(config.localdb_path().expect("memory path"), None);
}

#[test]
fn localdb_uses_an_explicit_path_without_environment_dependencies() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("sessions.db");
    let config = super::Config {
        storage: Storage::Localdb,
        localdb_path: Some(path.clone()),
        ..super::Config::default()
    };
    assert_eq!(config.localdb_path().expect("database path"), Some(path));
}

#[test]
fn storage_backend_is_explicit_and_rejects_a_path_in_memory_mode() {
    let args = crate::clap::Args::try_parse_from([
        "basrd",
        "--agents",
        "embedded",
        "--storage",
        "localdb",
        "--localdb-path",
        "sessions.sqlite3",
    ])
    .expect("localdb arguments");
    let config = args.config().expect("localdb config");
    assert_eq!(config.storage, Storage::Localdb);
    assert_eq!(
        config.localdb_path().expect("configured path"),
        Some("sessions.sqlite3".into())
    );

    let args = crate::clap::Args::try_parse_from([
        "basrd",
        "--agents",
        "embedded",
        "--storage",
        "memory",
        "--localdb-path",
        "sessions.sqlite3",
    ])
    .expect("parsed memory arguments");
    assert!(args.config().is_err());
}

#[test]
fn embedded_agents_do_not_silently_ignore_external_commands() {
    assert!(parse_agents("embedded=embedded:ignored-command").is_err());
}

#[test]
fn file_catalog_preserves_arguments_and_environment() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("agents.json");
    let agents = vec![crate::agent::AgentSpec::acp(
        "codex",
        crate::agent::Command::new(
            "/opt/agent tools/codex-acp",
            vec!["argument with spaces".into()],
        ),
    )];
    let mut configured = agents.clone();
    configured[0]
        .env
        .insert("CUSTOM_SETTING".into(), "a=b; c".into());
    std::fs::write(&path, serde_json::to_vec(&configured).expect("serialize")).expect("write");
    assert_eq!(super::read_agents(&path).expect("catalog"), configured);
}

#[test]
fn explicit_legacy_catalog_never_falls_back_when_missing_or_invalid() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("agents.json");
    assert!(super::read_agents(&path).is_err());
    for invalid in [
        "not json",
        "[]",
        r#"[{"id":"codex","kind":"acp"}]"#,
        r#"[{"id":"codex","kind":"acp","command":{"program":""}}]"#,
        r#"[{"id":"","kind":"embedded"}]"#,
        r#"[{"id":"embedded","kind":"embedded","command":{"program":"ignored"}}]"#,
        r#"[{"id":"codex","kind":"acp","command":{"program":"codex-acp","argz":[]}}]"#,
        r#"[{"id":"embedded","kind":"embedded","cancellation_grace_ms":0}]"#,
        r#"[{"id":"embedded","kind":"embedded","cancellation_grace_ms":-1}]"#,
        r#"[{"id":"embedded","kind":"embedded","cancellation_grace_ms":18446744073709551616}]"#,
    ] {
        std::fs::write(&path, invalid).expect("write invalid catalog");
        assert!(super::read_agents(&path).is_err(), "must reject {invalid}");
    }
}

#[test]
fn file_catalog_can_override_cancellation_grace_without_changing_the_default() {
    let directory = tempfile::tempdir().expect("test config directory");
    let path = directory.path().join("agents.json");
    std::fs::write(
        &path,
        r#"[
            {"id":"default","kind":"embedded"},
            {"id":"slow","kind":"acp","command":{"program":"agent"},"cancellation_grace_ms":12000}
        ]"#,
    )
    .expect("write test catalog");
    let agents = super::read_agents(&path).expect("valid catalog");
    assert_eq!(
        agents[0].cancellation_grace,
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        agents[1].cancellation_grace,
        std::time::Duration::from_secs(12)
    );
}
