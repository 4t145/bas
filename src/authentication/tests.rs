use serde_json::{Value, json};

use super::*;

fn initialize(methods: Value) -> Value {
    json!({"protocolVersion":1,"authMethods":methods,"agentCapabilities":{}})
}

#[test]
fn future_types_and_malformed_terminal_are_never_agent_fallback() {
    let init = initialize(json!([
        {"id":"future","name":"Future","type":"env_var","variable":"OPENAI_API_KEY"},
        {"id":"broken","name":"Broken","type":"terminal","args":"login"},
        {"id":"null","name":"Null","type":null},
        {"id":"env","name":"Env","type":"terminal","env":{"KEY":true}},
        {"id":"valid","name":"Valid"}
    ]));
    let result = methods(&init, None).expect("valid list");
    assert!(
        result[..4]
            .iter()
            .all(|method| matches!(method.execution, Execution::Unsupported { .. }))
    );
    assert_eq!(result[4].execution, Execution::Agent);
    assert_eq!(result[0].raw["variable"], "OPENAI_API_KEY");
}

#[test]
fn duplicate_and_invalid_descriptors_remain_disabled_individually() {
    let init = initialize(json!([
        {"id":"same","name":"First"},
        {"id":"same","name":"Second","type":"terminal"},
        null,
        {"id":"ok","name":"Working"}
    ]));
    let result = methods(&init, None).expect("valid list");
    assert!(
        result[..3]
            .iter()
            .all(|method| matches!(method.execution, Execution::Unsupported { .. }))
    );
    assert_eq!(result[3].execution, Execution::Agent);
}

#[test]
fn invalid_terminal_env_and_nul_are_disabled_without_erasing_raw() {
    let init = initialize(json!([
        {"id":"nul","name":"Nul","type":"terminal","args":["a\u{0000}b"]},
        {"id":"equals","name":"Equals","type":"terminal","env":{"BAD=KEY":"value"}},
        {"id":"legacy","name":"Legacy","_meta":{"terminal-auth":{"command":"","args":["login"]}}}
    ]));
    assert!(
        methods(&init, None)
            .expect("valid list")
            .iter()
            .all(|method| matches!(method.execution, Execution::Unsupported { .. }))
    );
}

#[test]
fn legacy_application_authentication_depends_on_format_not_agent_identity() {
    // Shape emitted by Qwen Code's buildAuthMethods; identity is presentation data.
    let descriptor = json!({
        "id": "configure-provider",
        "name": "Configure provider",
        "_meta": {"type": "terminal", "args": ["--auth-type=openai"]}
    });
    for info in [
        Value::Null,
        json!({"name": "qwen-code", "version": "0.25.0"}),
        json!({"name": "qwen-code", "version": "1.0.0"}),
        json!({"name": "another-agent", "version": "custom"}),
    ] {
        let mut init = initialize(json!([descriptor.clone()]));
        init["agentInfo"] = info;
        let error = json!({
            "code": -32000,
            "data": {"authMethods": [descriptor.clone()]}
        });
        for error in [None, Some(&error)] {
            let normalized = methods(&init, error).expect("valid method list");
            assert_eq!(normalized[0].raw, descriptor);
            assert_eq!(
                normalized[0].execution,
                Execution::Terminal(Terminal {
                    args: vec!["--auth-type=openai".into()],
                    env: BTreeMap::new(),
                    arguments: Arguments::ReplaceApplication,
                    command: None,
                })
            );
            assert_eq!(
                normalized[0].source,
                if error.is_some() {
                    Source::AuthenticationError
                } else {
                    Source::Initialize
                }
            );
        }
    }
}

#[test]
fn legacy_application_fields_are_validated_without_guessed_commands() {
    for meta in [
        json!({"type": "future"}),
        json!({"type": null}),
        json!({"type": "terminal", "command": "/different/program"}),
        json!({"type": "terminal", "args": "login"}),
        json!({"type": "terminal", "args": ["bad\u{0000}arg"]}),
        json!({"type": "terminal", "env": {"BAD=KEY": "value"}}),
    ] {
        let raw = json!({"id": "login", "name": "Login", "_meta": meta});
        let normalized = methods(&initialize(json!([raw.clone()])), None)
            .expect("invalid method remains visible");
        assert!(matches!(
            normalized[0].execution,
            Execution::Unsupported { .. }
        ));
        assert_eq!(normalized[0].raw, raw);
    }
    let init = initialize(json!([{
        "id": "login",
        "name": "Login",
        "_meta": {
            "type": "terminal",
            "args": ["configure", "literal $argument"],
            "env": {"MODE": "interactive"}
        }
    }]));
    let normalized = methods(&init, None).expect("valid legacy format");
    let Execution::Terminal(terminal) = &normalized[0].execution else {
        panic!("legacy terminal");
    };
    assert_eq!(terminal.args, ["configure", "literal $argument"]);
    assert_eq!(terminal.env["MODE"], "interactive");
}

#[test]
fn standard_discriminators_take_precedence_over_legacy_metadata() {
    let init = initialize(json!([
        {"id": "agent", "name": "Agent", "type": "agent", "_meta": {"type": "terminal"}},
        {
            "id": "terminal", "name": "Terminal", "type": "terminal", "args": ["--login"],
            "_meta": {"type": "terminal", "args": ["ignored"]}
        },
        {"id": "future", "name": "Future", "type": "future", "_meta": {"type": "terminal"}},
        {"id": "ordinary", "name": "Ordinary", "_meta": {"color": "blue"}}
    ]));
    let normalized = methods(&init, None).expect("valid list");
    assert_eq!(normalized[0].execution, Execution::Agent);
    let Execution::Terminal(terminal) = &normalized[1].execution else {
        panic!("standard terminal");
    };
    assert_eq!(terminal.arguments, Arguments::Append);
    assert_eq!(terminal.args, ["--login"]);
    assert!(matches!(
        normalized[2].execution,
        Execution::Unsupported { .. }
    ));
    assert_eq!(normalized[3].execution, Execution::Agent);
}

#[test]
fn matching_error_method_retains_the_initial_full_invocation_contract() {
    let init = initialize(json!([{
        "id": "login", "name": "Login",
        "_meta": {"terminal-auth": {"command": "agent", "args": ["login"]}}
    }]));
    for (args, expected) in [
        (vec!["login"], Arguments::ReplaceInvocation),
        (vec!["login", "--different"], Arguments::Append),
    ] {
        let error = json!({
            "code": -32000,
            "data": {"authMethods": [{
                "id": "login", "name": "Login", "type": "terminal", "args": args
            }]}
        });
        let normalized = methods(&init, Some(&error)).expect("authentication error methods");
        let Execution::Terminal(terminal) = &normalized[0].execution else {
            panic!("terminal method");
        };
        assert_eq!(terminal.arguments, expected);
        assert_eq!(terminal.args, args);
        assert_eq!(
            terminal.command.as_deref(),
            (expected == Arguments::ReplaceInvocation).then_some("agent")
        );
    }
}
