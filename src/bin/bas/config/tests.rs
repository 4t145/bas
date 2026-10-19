use clap::Parser;

use super::{Notify, Storage, parse_agents};

#[test]
fn defaults_are_suitable_for_a_local_daemon() {
    let config = super::Config::default();

    assert!(config.listen.ip().is_loopback());
    assert_eq!(config.notify, Notify::Auto);
    assert_eq!(config.storage, Storage::Localdb);
    assert_eq!(config.localdb_path, None);
    assert_eq!(
        config.agents,
        vec![crate::agent::AgentSpec::embedded().into()]
    );
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
fn storage_backend_is_explicit_and_rejects_a_path_in_memory_mode() {
    let args = crate::clap::Args::try_parse_from([
        "bas",
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
        "bas",
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
