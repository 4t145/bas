use super::{Command, DEFAULT_PLAN_MILLIS, DEFAULT_SLEEP_MILLIS, StopReason, parse};

#[test]
fn plain_prompts_are_echoed_and_slash_commands_are_parsed() {
    assert_eq!(
        parse("  hello world  "),
        Command::Echo {
            text: "hello world".to_string()
        }
    );
    assert_eq!(parse("/sleep 25"), Command::Sleep { millis: 25 });
    assert_eq!(parse("/plan 25"), Command::Plan { millis: 25 });
    assert_eq!(
        parse("/plan"),
        Command::Plan {
            millis: DEFAULT_PLAN_MILLIS
        }
    );
    assert_eq!(parse("/plan clear"), Command::ClearPlan);
    assert_eq!(
        parse("/sleep"),
        Command::Sleep {
            millis: DEFAULT_SLEEP_MILLIS
        }
    );
    assert_eq!(
        parse("/stop max_tokens"),
        Command::Stop {
            reason: StopReason::MaxTokens
        }
    );
    assert_eq!(
        parse("/unknown thing"),
        Command::Unknown("unknown".to_string())
    );
}

#[test]
fn invalid_arguments_are_reported_instead_of_silently_defaulting() {
    assert!(matches!(parse("/sleep soon"), Command::Invalid { .. }));
    assert!(matches!(parse("/plan soon"), Command::Invalid { .. }));
    assert!(matches!(parse("/stop eventually"), Command::Invalid { .. }));
}
