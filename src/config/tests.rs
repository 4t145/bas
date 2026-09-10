use std::str::FromStr;

use crate::agent::Kind;

use super::{Notify, parse_agents};

#[test]
fn parses_the_embedded_agent() {
    let agents = parse_agents("embedded").unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].kind, Kind::Embedded);
    assert_eq!(agents[0].id.as_str(), "embedded");
    assert!(agents[0].command().is_none());
}

#[test]
fn parses_acp_agents_with_arguments() {
    let agents = parse_agents(" embedded ; opencode=acp:opencode acp --print-logs ").unwrap();

    assert_eq!(agents.len(), 2);
    let acp = &agents[1];
    assert_eq!(acp.kind, Kind::Acp);
    assert_eq!(acp.id.as_str(), "opencode");
    let command = acp.command().unwrap();
    assert_eq!(command.program.to_string_lossy(), "opencode");
    assert_eq!(command.args, vec!["acp", "--print-logs"]);
}

#[test]
fn rejects_an_acp_agent_without_a_command() {
    let error = parse_agents("opencode=acp").unwrap_err();

    assert!(error.to_string().contains("has no command line"));
}

#[test]
fn rejects_an_unknown_kind() {
    let error = parse_agents("thing=telepathy").unwrap_err();

    assert!(error.to_string().contains("unknown agent kind"));
}

#[test]
fn parses_notification_policies() {
    assert_eq!(Notify::from_str(" AUTO ").unwrap(), Notify::Auto);
    assert_eq!(Notify::from_str("always").unwrap(), Notify::Always);
    assert_eq!(Notify::from_str("off").unwrap(), Notify::Never);
    assert!(Notify::from_str("sometimes").is_err());
}

#[test]
fn defaults_are_suitable_for_a_local_daemon() {
    let config = super::Config::default();

    assert!(config.listen.ip().is_loopback());
    assert_eq!(config.notify, Notify::Auto);
    assert_eq!(config.agents, vec![crate::agent::AgentSpec::embedded()]);
}
