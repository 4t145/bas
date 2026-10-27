use super::SessionId;

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
