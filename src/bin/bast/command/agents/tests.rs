use bas::authentication::elicitation::{Mode, parse_request};
use serde_json::json;

use super::parse_input;

#[test]
fn login_completion_requires_connected_readiness() {
    use bas::service::pb::{AgentCheckStatus, CheckAgentResponse};

    let ready = CheckAgentResponse {
        status: AgentCheckStatus::Connected.into(),
        ..Default::default()
    };
    assert!(super::finish_login("test-agent", &ready).is_ok());

    for status in [
        AgentCheckStatus::AuthenticationRequired,
        AgentCheckStatus::Unspecified,
    ] {
        let incomplete = CheckAgentResponse {
            status: status.into(),
            ..Default::default()
        };
        let error = super::finish_login("test-agent", &incomplete)
            .expect_err("authentication must not succeed without readiness");
        assert_eq!(
            error.to_string(),
            "Agent setup is incomplete; review its native configuration"
        );
    }
}

#[test]
fn terminal_authentication_requires_same_machine_ipc() {
    for value in ["local", "unix:///tmp/bas.sock", "pipe://bas-test"] {
        let target = value.parse().expect("valid target");
        assert!(super::require_local_terminal(&target).is_ok(), "{value}");
    }
    for value in ["tcp://127.0.0.1:4554", "tcp://192.0.2.1:4554", "[::1]:4554"] {
        let remote = value.parse().expect("remote target");
        assert!(super::require_local_terminal(&remote).is_err());
    }
}

#[test]
fn terminal_choices_return_agent_values_and_validate_constraints() {
    let request = parse_request(&json!({
        "mode":"form","requestId":"configure","message":"Project settings",
        "requestedSchema":{"type":"object","properties":{
            "model":{"type":"string","enum":["small","large"]},
            "count":{"type":"integer","minimum":1},
            "enabled":{"type":"boolean"}
        }}
    }))
    .expect("supported schema");
    let Mode::Form(form) = request.mode else {
        panic!("form");
    };
    let field = |name: &str| {
        form.fields
            .iter()
            .find(|field| field.name == name)
            .expect("field")
    };
    assert_eq!(
        parse_input(field("model"), "2").expect("choice"),
        Some(json!("large"))
    );
    assert!(parse_input(field("model"), "3").is_err());
    let zero = parse_input(field("count"), "0")
        .expect("number")
        .expect("value");
    assert!(field("count").validate(&zero).is_err());
    let fraction = parse_input(field("count"), "1.5")
        .expect("number")
        .expect("value");
    assert!(field("count").validate(&fraction).is_err());
    assert_eq!(
        parse_input(field("enabled"), "false").expect("boolean"),
        Some(json!(false))
    );
    assert_eq!(
        parse_input(field("model"), "").expect("optional omission"),
        None
    );
}
