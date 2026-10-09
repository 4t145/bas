use super::*;
use basr::acp::{SessionConfigOption, SessionConfigOptionCategory};

#[test]
fn boolean_tooltips_include_the_backend_name_current_state_and_description() {
    let actual = SessionConfigOption::boolean("fast-mode", "Fast mode", false)
        .category(SessionConfigOptionCategory::ModelConfig)
        .description("1.5x speed, increased usage");
    assert_eq!(
        config_option_hint(&actual),
        "Fast mode · 关闭 · 1.5x speed, increased usage"
    );
    let enabled = SessionConfigOption::boolean("another-backend-option", "Backend feature", true);
    assert_eq!(config_option_hint(&enabled), "Backend feature · 开启");
}
