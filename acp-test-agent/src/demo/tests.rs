use super::{MODEL_DEEP, MODEL_FAST, SessionConfig};

#[test]
fn config_settings_change_only_the_selected_option() {
    let mut config = SessionConfig::default();
    for (setting, thinking) in [
        (None, true),
        (Some("toggle"), false),
        (Some("on"), true),
        (Some("off"), false),
    ] {
        config.apply_setting(setting).expect("supported toggle");
        assert_eq!(config.thinking, thinking);
        assert_eq!(config.model, MODEL_FAST);
    }
    for (setting, model) in [("model=deep", MODEL_DEEP), ("fast", MODEL_FAST)] {
        config
            .apply_setting(Some(setting))
            .expect("supported model");
        assert_eq!(config.model, model);
        assert!(!config.thinking);
    }
}

#[test]
fn invalid_config_setting_preserves_the_existing_values() {
    let mut config = SessionConfig {
        model: MODEL_DEEP.into(),
        thinking: true,
    };
    let before = config.clone();
    for setting in ["model=unknown", "thinking=on", ""] {
        assert!(config.apply_setting(Some(setting)).is_err());
        assert_eq!(config, before);
    }
}
