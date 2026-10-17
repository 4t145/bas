use super::{SessionConfig, apply_config_setting, help_text};

#[test]
fn help_lists_every_advertised_command() {
    let help = help_text();
    for command in super::COMMANDS {
        assert!(help.contains(&format!("/{}", command.name)));
    }
}

#[test]
fn help_renders_as_a_markdown_list() {
    let help = help_text();
    assert!(help.starts_with("ACP test agent demos:\n\n- `/"), "{help}");
    assert_eq!(
        help.lines()
            .filter(|line| line.starts_with("- `/") && line.contains(" — "))
            .count(),
        super::COMMANDS.len()
    );
}

#[test]
fn config_settings_update_values_and_reject_unknown_names() {
    let mut config = SessionConfig::default();
    apply_config_setting(&mut config, None).unwrap();
    assert!(config.thinking);
    apply_config_setting(&mut config, Some("off")).unwrap();
    assert!(!config.thinking);
    apply_config_setting(&mut config, Some("model=deep")).unwrap();
    assert_eq!(config.model, "deep");
    assert!(apply_config_setting(&mut config, Some("model=unknown")).is_err());
}
