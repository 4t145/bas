use std::str::FromStr;

use super::{Cursor, SessionId};

#[test]
fn parses_and_prints_session_ids() {
    let id = SessionId::new();
    let text = id.to_string();

    assert_eq!(SessionId::from_str(&text).unwrap(), id);
    assert!(SessionId::from_str("not-a-uuid").is_err());
    assert!(
        SessionId::from_str("not-a-uuid")
            .unwrap_err()
            .to_string()
            .contains("session id")
    );
}

#[test]
fn cursors_increase_by_one() {
    assert_eq!(Cursor::START.next().raw(), 1);
    assert_eq!(Cursor::START.next().next().raw(), 2);
    assert_eq!(Cursor::from_raw(41).raw(), 41);
}

#[test]
fn acp_configuration_updates_are_available_in_session_metadata() {
    use crate::acp;
    let mut meta =
        super::SessionMeta::new(SessionId::new(), "test".into(), super::Owner::current());
    let options = vec![acp::SessionConfigOption::boolean(
        "enabled", "Enabled", true,
    )];
    meta.apply_acp(&acp::SessionUpdate::ConfigOptionUpdate(
        acp::ConfigOptionUpdate::new(options.clone()),
    ));
    assert_eq!(meta.acp.expect("ACP state").config_options, Some(options));
}
