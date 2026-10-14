use super::*;
use bas::acp::{SessionConfigOption, SessionConfigOptionCategory};

#[test]
fn boolean_tooltips_describe_only_the_next_action() {
    let actual = SessionConfigOption::boolean("fast-mode", "Fast mode", false)
        .category(SessionConfigOptionCategory::ModelConfig)
        .description("1.5x speed, increased usage");
    assert_eq!(
        crate::i18n::visible_text(&config_option_hint(&actual)),
        "开启Fast mode"
    );
    let enabled = SessionConfigOption::boolean("another-backend-option", "Backend feature", true);
    assert_eq!(
        crate::i18n::visible_text(&config_option_hint(&enabled)),
        "关闭Backend feature"
    );
}
