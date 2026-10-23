use super::{Command, parse};

#[test]
fn invalid_arguments_are_reported_instead_of_silently_defaulting() {
    assert!(matches!(parse("/sleep soon"), Command::Invalid { .. }));
    assert!(matches!(parse("/plan soon"), Command::Invalid { .. }));
    assert!(matches!(parse("/stop eventually"), Command::Invalid { .. }));
}
