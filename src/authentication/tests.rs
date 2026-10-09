use serde_json::{Value, json};

use super::*;

fn initialize(methods: Value) -> Value {
    json!({"protocolVersion":1,"authMethods":methods,"agentCapabilities":{}})
}

fn auth_required(methods: Value) -> Value {
    json!({"code":-32000,"message":"Authentication required","data":{"authMethods":methods}})
}

fn kimi_method() -> Value {
    // MoonshotAI/kimi-cli 1.52.0, src/kimi_cli/acp/server.py.
    json!({
        "id":"login","name":"Login with Kimi account",
        "description":"Run `kimi login` command in the terminal, then follow the instructions to finish login.",
        "_meta":{"terminal-auth":{
            "command":"/agents/kimi","args":["login"],"label":"Kimi Code Login","env":{},"type":"terminal"
        }}
    })
}

#[test]
fn standard_types_preserve_raw_and_ignore_display_text() {
    let raw = json!({"id":"api-key","name":"Login in terminal","type":"agent","_meta":{"terminal-auth":{"command":"other","args":["login"]}},"future":{"x":true}});
    let init = initialize(json!([raw.clone(), {"id":"browser","name":"浏览器认证"}]));
    let result = methods(&init, None).expect("valid methods");
    assert_eq!(result[0].execution, Execution::Agent);
    assert_eq!(result[1].execution, Execution::Agent);
    assert_eq!(result[0].raw, raw);
    assert_eq!(result[0].source, Source::Initialize);
}

#[test]
fn standard_terminal_appends_and_accepts_literal_arguments() {
    let init = initialize(
        json!([{"id":"login","name":"Continue","type":"terminal","args":["--auth","$(touch /tmp/no)",""],"env":{"TOKEN":"literal $VALUE"}}]),
    );
    let result = methods(&init, None).expect("valid methods");
    assert_eq!(
        result[0].execution,
        Execution::Terminal(Terminal {
            args: vec!["--auth".into(), "$(touch /tmp/no)".into(), "".into()],
            env: BTreeMap::from([("TOKEN".into(), "literal $VALUE".into())]),
            arguments: Arguments::Append,
            command: None,
        })
    );
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
fn active_error_list_replaces_initial_list_including_explicit_empty() {
    let init = initialize(json!([{"id":"old","name":"Old"}]));
    let error = auth_required(json!([{"id":"new","name":"New"}]));
    let result = methods(&init, Some(&error)).expect("valid methods");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, "new");
    assert_eq!(result[0].source, Source::AuthenticationError);
    let empty = auth_required(json!([]));
    assert!(
        methods(&init, Some(&empty))
            .expect("explicit empty list")
            .is_empty()
    );
    let malformed = auth_required(json!({"id":"new"}));
    assert!(methods(&init, Some(&malformed)).is_err());
}

#[test]
fn unrelated_errors_cannot_replace_advertised_methods() {
    let init = initialize(json!([{"id":"old","name":"Old"}]));
    let error = json!({"code":-32603,"data":{"authMethods":[{"id":"new","name":"New"}]}});
    let result = methods(&init, Some(&error)).expect("valid methods");
    assert_eq!(result[0].id, "old");
    assert_eq!(result[0].source, Source::Initialize);
}

#[test]
fn kimi_full_descriptor_and_matching_flattened_error_share_legacy_semantics() {
    let init = initialize(json!([kimi_method()]));
    let initial = methods(&init, None).expect("valid legacy method");
    let flattened = json!({"id":"login","name":"Login with Kimi account","type":"terminal","args":["login"],"env":{}});
    let error = auth_required(json!([flattened.clone()]));
    let active = methods(&init, Some(&error)).expect("valid error method");
    assert_eq!(active[0].execution, initial[0].execution);
    assert_eq!(active[0].raw, flattened);
    assert_eq!(active[0].source, Source::AuthenticationError);
    assert!(matches!(
        active[0].execution,
        Execution::Terminal(Terminal {
            arguments: Arguments::ReplaceInvocation,
            ..
        })
    ));
}

#[test]
fn error_descriptor_conflict_does_not_inherit_legacy_operation() {
    let init = initialize(json!([kimi_method()]));
    let error = auth_required(
        json!([{"id":"login","name":"Different","type":"terminal","args":["other"],"env":{}}]),
    );
    let active = methods(&init, Some(&error)).expect("valid standard descriptor");
    assert!(
        matches!(&active[0].execution, Execution::Terminal(Terminal { arguments:Arguments::Append, command:None, args, .. }) if args == &["other"])
    );
}

#[test]
fn qwen_verified_metadata_replaces_application_args_without_name_inference() {
    // QwenLM/qwen-code v0.25.0, authMethods.ts and installed bundle initialize.
    let raw = json!({"id":"openai","name":"Use OpenAI API key","description":"Requires setting the `OPENAI_API_KEY` environment variable","_meta":{"type":"terminal","args":["--auth-type=openai"]}});
    let mut init = initialize(json!([raw.clone()]));
    init["agentInfo"] = json!({"name":"qwen-code","version":"0.25.0"});
    let result = methods(&init, None).expect("valid contract");
    assert!(
        matches!(&result[0].execution, Execution::Terminal(Terminal {arguments:Arguments::ReplaceApplication,args,..}) if args == &["--auth-type=openai"])
    );
    init["agentInfo"]["version"] = json!("9.0.0");
    let result = methods(&init, None).expect("valid list");
    assert!(matches!(result[0].execution, Execution::Unsupported { .. }));
    init["authMethods"][0]["type"] = json!("agent");
    let result = methods(&init, None).expect("explicit standard wins");
    assert_eq!(result[0].execution, Execution::Agent);
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
