use std::str::FromStr;

use super::{Backend, Config};

#[test]
fn parses_backend_names_case_insensitively() {
    assert_eq!(Backend::from_str("auto").unwrap(), Backend::Auto);
    assert_eq!(Backend::from_str(" STDOUT ").unwrap(), Backend::Stdout);
    assert_eq!(Backend::from_str("Journal").unwrap(), Backend::Journald);
    assert_eq!(Backend::from_str("journald").unwrap(), Backend::Journald);
}

#[test]
fn rejects_unknown_backend_name() {
    let error = Backend::from_str("file").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("expected auto, stdout, or journald")
    );
}

#[test]
fn default_is_safe_for_service_output() {
    let config = Config::default();

    assert_eq!(config.backend(), Backend::Auto);
    assert_eq!(config.filter(), "info");
    assert!(!config.ansi());
    assert!(config.includes_target());
    assert_eq!(config.syslog_identifier(), None);
}

#[test]
fn builder_updates_backend_options() {
    let config = Config::new(Backend::Journald)
        .with_filter("basr=debug")
        .with_ansi(true)
        .with_target(false)
        .with_syslog_identifier("basrd");

    assert_eq!(config.backend(), Backend::Journald);
    assert_eq!(config.filter(), "basr=debug");
    assert!(config.ansi());
    assert!(!config.includes_target());
    assert_eq!(config.syslog_identifier(), Some("basrd"));
}

#[test]
fn validates_filter_before_global_initialization() {
    assert!(Config::default().env_filter().is_ok());
    assert!(
        Config::default()
            .with_filter("basr==debug")
            .env_filter()
            .is_err()
    );
}

#[test]
fn auto_uses_journald_only_when_supported_and_connected() {
    assert_eq!(Backend::for_environment(true, true), Backend::Journald);
    assert_eq!(Backend::for_environment(false, true), Backend::Stdout);
    assert_eq!(Backend::for_environment(true, false), Backend::Stdout);
    assert_eq!(Backend::for_environment(false, false), Backend::Stdout);
}
