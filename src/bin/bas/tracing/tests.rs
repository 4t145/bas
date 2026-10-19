use super::{Backend, Config};

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
fn validates_filter_before_global_initialization() {
    assert!(Config::default().env_filter().is_ok());
    assert!(
        Config::default()
            .with_filter("bas==debug")
            .env_filter()
            .is_err()
    );
}
