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
