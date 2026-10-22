use clap::Parser;

use super::{Notify, Storage, parse_agents};

#[test]
fn agent_overrides_still_load_and_watch_server_policy() {
    let directory = tempfile::tempdir().expect("settings directory");
    let settings = directory.path().join("settings.json");
    let legacy = directory.path().join("agents.json");
    let spec = crate::agent::AgentSpec::acp("a", crate::agent::Command::new("a", []));
    let mut document = bas::config::Settings::default();
    document.daemon.agents = vec![spec.clone().into()];
    document.daemon.acp.pool_size = 1;
    document.daemon.acp.default_agent = Some("a".into());
    std::fs::write(&settings, serde_json::to_vec(&document).expect("JSON")).expect("settings");
    std::fs::write(&legacy, serde_json::to_vec(&vec![spec]).expect("JSON")).expect("legacy");
    for (flag, value) in [
        ("--agents", "a=acp:a"),
        ("--agents-file", legacy.to_str().expect("path")),
    ] {
        let args = crate::clap::Args::try_parse_from([
            "bas",
            "--daemon-settings-file",
            settings.to_str().expect("path"),
            flag,
            value,
        ])
        .expect("arguments");
        let config = args.config().expect("config");
        assert_eq!(config.acp, document.daemon.acp);
        let mut source = config.catalog_source.expect("policy remains reloadable");
        assert!(
            source.settings_store().is_none(),
            "overridden catalogs are not persisted"
        );
        document.daemon.acp.pool_size = 2;
        std::fs::write(&settings, serde_json::to_vec(&document).expect("JSON")).expect("update");
        assert_eq!(
            source.load_configuration().expect("reload").acp.pool_size,
            2
        );
    }
}

#[test]
fn defaults_are_suitable_for_a_local_daemon() {
    let config = super::Config::default();

    assert_eq!(config.listen, vec![bas::service::Target::Local]);
    assert_eq!(config.notify, Notify::Auto);
    assert_eq!(config.storage, Storage::Localdb);
    assert_eq!(config.localdb_path, None);
    assert_eq!(
        config.agents,
        vec![crate::agent::AgentSpec::embedded().into()]
    );
}

#[test]
fn listeners_default_to_ipc_and_tcp_must_be_selected() {
    use bas::service::Target;

    let args = crate::clap::Args::try_parse_from(["bas", "--agents", "embedded"])
        .expect("default arguments");
    assert_eq!(args.config().unwrap().listen, vec![Target::Local]);

    let args = crate::clap::Args::try_parse_from([
        "bas",
        "--agents",
        "embedded",
        "--listen",
        "local",
        "--listen",
        "tcp://127.0.0.1:0",
    ])
    .expect("multiple listeners");
    assert_eq!(
        args.config().unwrap().listen,
        vec![Target::Local, Target::Tcp("127.0.0.1:0".parse().unwrap())]
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
