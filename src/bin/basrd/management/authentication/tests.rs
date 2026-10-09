use super::*;
use basr::{
    authentication::{Source, Terminal},
    config::agent::Command,
};
use serde_json::json;

fn method(arguments: Arguments, command: Option<String>) -> Method {
    Method {
        id: "setup".into(),
        name: "Configure agent".into(),
        description: None,
        source: Source::Initialize,
        raw: json!({"id":"setup","name":"Configure agent"}),
        execution: Execution::Terminal(Terminal {
            args: vec!["--auth-type=openai".into()],
            env: BTreeMap::from([("MODE".into(), "interactive".into())]),
            arguments,
            command,
        }),
    }
}

#[test]
fn standard_terminal_appends_literal_arguments_and_overrides_environment() {
    let mut spec = AgentSpec::acp(
        "fixture",
        Command::new(
            "/fixture/agent",
            ["--acp", "literal $arg"].map(str::to_owned),
        ),
    );
    spec.env.insert("MODE".into(), "acp".into());
    let plan = build(
        &spec,
        Path::new("/fixture/agent"),
        &method(Arguments::Append, None),
        None,
    )
    .expect("standard invocation");
    assert_eq!(plan.args, ["--acp", "literal $arg", "--auth-type=openai"]);
    assert_eq!(plan.env["MODE"], "interactive");
}

#[test]
fn installed_node_entrypoint_survives_legacy_application_argument_replacement() {
    let directory = tempfile::tempdir().expect("installation directory");
    let node = directory.path().join("node");
    let entry = directory.path().join("cli-entry.js");
    std::fs::write(&node, "runtime").expect("node placeholder");
    std::fs::write(&entry, "entrypoint").expect("entrypoint placeholder");
    let spec = AgentSpec::acp(
        "fixture",
        Command::new(
            &node,
            [
                entry.to_string_lossy().into_owned(),
                "--acp".into(),
                "--experimental-skills".into(),
            ],
        ),
    );
    let record = Record {
        manifest: serde_json::from_value(json!({"id":"fixture","name":"Fixture","version":"1.0.0","distribution":{"npx":{"package":"fixture@1.0.0","args":["--acp","--experimental-skills"]}}})).expect("manifest"),
        directory: directory.path().to_owned(), spec: spec.clone(),
        runtime: Some(super::super::runtime::Runtime {node: node.clone(),npm_cli: directory.path().join("npm-cli.js"),version:"1.0.0".into()}),
        dependencies: BTreeMap::new(), launch: None,
    };
    let plan = build(
        &spec,
        &node,
        &method(Arguments::ReplaceApplication, None),
        Some(&record),
    )
    .expect("known installed launcher");
    assert_eq!(plan.program, node);
    assert_eq!(
        plan.args,
        [
            entry.to_string_lossy().into_owned(),
            "--auth-type=openai".into()
        ]
    );
    let mut changed = spec;
    changed
        .command
        .as_mut()
        .expect("command")
        .args
        .push("--changed".into());
    assert!(
        build(
            &changed,
            &plan.program,
            &method(Arguments::ReplaceApplication, None),
            Some(&record)
        )
        .is_err()
    );
}

#[test]
fn custom_wrappers_require_an_explicit_trusted_application_boundary() {
    let spec = AgentSpec::acp(
        "custom",
        Command::new(
            "/fixture/node",
            ["entrypoint.js", "--acp"].map(str::to_owned),
        ),
    );
    assert!(
        build(
            &spec,
            Path::new("/fixture/node"),
            &method(Arguments::ReplaceApplication, None),
            None
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn native_legacy_login_uses_complete_arguments_instead_of_acp_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("installation directory");
    let program = directory.path().join("kimi");
    std::fs::write(&program, "#!/bin/sh\nexit 0\n").expect("agent executable");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
        .expect("executable permissions");
    let spec = AgentSpec::acp("kimi", Command::new(&program, ["acp".to_owned()]));
    let record = Record {
        manifest: serde_json::from_value(json!({"id":"kimi","name":"Kimi","version":"1.52.0","distribution":{"binary":{super::super::registry::platform():{"archive":"https://example.com/kimi.tar.gz","cmd":"kimi","args":["acp"]}}}})).expect("manifest"),
        directory: directory.path().to_owned(), spec: spec.clone(), runtime: None, dependencies: BTreeMap::new(), launch: None,
    };
    let mut login = method(
        Arguments::ReplaceInvocation,
        Some(program.to_string_lossy().into_owned()),
    );
    if let Execution::Terminal(terminal) = &mut login.execution {
        terminal.args = vec!["login".into()];
    }
    let plan =
        build(&spec, &program, &login, Some(&record)).expect("complete native login command");
    assert_eq!(plan.program, program);
    assert_eq!(plan.args, ["login"]);
}

#[cfg(unix)]
#[test]
fn legacy_node_descriptor_preserves_exactly_one_entrypoint() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("installation directory");
    let node = directory.path().join("node");
    let entry = directory.path().join("entry.js");
    std::fs::write(&node, "#!/bin/sh\nexit 0\n").expect("node executable");
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755))
        .expect("executable permissions");
    std::fs::write(&entry, "entrypoint").expect("entrypoint");
    let spec = AgentSpec::acp(
        "fixture",
        Command::new(
            &node,
            [entry.to_string_lossy().into_owned(), "--acp".into()],
        ),
    );
    let record = Record {
        manifest: serde_json::from_value(json!({"id":"fixture","name":"Fixture","version":"1.0.0","distribution":{"npx":{"package":"fixture@1.0.0","args":["--acp"]}}})).expect("manifest"),
        directory: directory.path().to_owned(), spec: spec.clone(),
        runtime: Some(super::super::runtime::Runtime {node: node.clone(),npm_cli: directory.path().join("npm-cli.js"),version:"1.0.0".into()}),
        dependencies: BTreeMap::new(), launch: None,
    };
    let mut login = method(
        Arguments::ReplaceInvocation,
        Some(node.to_string_lossy().into_owned()),
    );
    if let Execution::Terminal(terminal) = &mut login.execution {
        terminal.args = vec![entry.to_string_lossy().into_owned(), "login".into()];
    }
    let plan = build(&spec, &node, &login, Some(&record)).expect("complete node login command");
    assert_eq!(
        plan.args,
        [entry.to_string_lossy().into_owned(), "login".into()]
    );
    if let Execution::Terminal(terminal) = &mut login.execution {
        terminal.args = vec!["login".into()];
    }
    assert!(build(&spec, &node, &login, Some(&record)).is_err());
}
