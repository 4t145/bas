use super::*;
use bas::{
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

#[test]
fn structural_legacy_descriptor_keeps_the_verified_npm_entrypoint() {
    let directory = tempfile::tempdir().expect("installation directory");
    let entrypoint = directory.path().join("entrypoint.js");
    std::fs::write(&entrypoint, "// fixture").expect("installed entrypoint");
    let node = directory.path().join("node");
    let spec = AgentSpec::acp(
        "fixture",
        Command::new(
            &node,
            [entrypoint.to_string_lossy().into_owned(), "--acp".into()],
        ),
    );
    let record = Record {
        manifest: serde_json::from_value(json!({
            "id": "fixture", "name": "Fixture", "version": "1.0.0",
            "distribution": {"npx": {"package": "fixture@1.0.0", "args": ["--acp"]}}
        }))
        .expect("manifest"),
        directory: directory.path().to_owned(),
        spec: spec.clone(),
        runtime: Some(super::super::runtime::Runtime {
            node: node.clone(),
            npm_cli: directory.path().join("npm.js"),
            version: "fixture".into(),
        }),
        dependencies: BTreeMap::new(),
        launch: None,
    };
    let initialized = json!({
        "authMethods": [{
            "id": "native-login", "name": "Native login",
            "_meta": {"type": "terminal", "args": ["--auth-type=openai"]}
        }]
    });
    let methods = bas::authentication::methods(&initialized, None).expect("legacy descriptor");
    let plan = build(&spec, &node, &methods[0], Some(&record)).expect("verified launch boundary");
    assert_eq!(plan.program, node);
    assert_eq!(
        plan.args,
        [entrypoint.to_string_lossy().as_ref(), "--auth-type=openai"]
    );
    assert!(
        build(&spec, &node, &methods[0], None).is_err(),
        "the same descriptor cannot authorize an unverified custom wrapper"
    );
}

#[test]
fn boundary_matching_requires_the_exact_program_and_ordered_argument_sequence() {
    let boundary = LaunchBoundary {
        program: PathBuf::from("/fixture/node"),
        application: PathBuf::from("/fixture/entrypoint.js"),
        bootstrap_args: vec!["/fixture/entrypoint.js".into()],
        application_args: vec!["--acp".into(), "literal $arg".into()],
    };
    for (program, args, expected) in [
        (
            "/fixture/node",
            vec!["/fixture/entrypoint.js", "--acp", "literal $arg"],
            true,
        ),
        (
            "/other/node",
            vec!["/fixture/entrypoint.js", "--acp", "literal $arg"],
            false,
        ),
        ("/fixture/node", vec!["--acp", "literal $arg"], false),
        (
            "/fixture/node",
            vec!["/fixture/entrypoint.js", "literal $arg", "--acp"],
            false,
        ),
        (
            "/fixture/node",
            vec!["/fixture/entrypoint.js", "--acp", "literal $arg", "extra"],
            false,
        ),
    ] {
        let spec = AgentSpec::acp(
            "fixture",
            Command::new(program, args.into_iter().map(str::to_owned)),
        );
        assert_eq!(boundary.matches(&spec), expected, "{spec:?}");
    }
}

#[test]
fn checked_boundary_rejects_edited_specs_and_stored_launch_metadata() {
    let directory = tempfile::tempdir().expect("installation directory");
    let program = directory.path().join("agent");
    let spec = AgentSpec::acp("fixture", Command::new(&program, ["--acp".into()]));
    let mut record = Record {
        manifest: serde_json::from_value(json!({
            "id": "fixture",
            "name": "Fixture",
            "version": "1.0.0",
            "distribution": {"binary": {registry::platform(): {
                "archive": "https://example.com/agent.tar.gz",
                "cmd": "agent",
                "args": ["--acp"]
            }}}
        }))
        .expect("manifest"),
        directory: directory.path().to_owned(),
        spec: spec.clone(),
        runtime: None,
        dependencies: BTreeMap::new(),
        launch: None,
    };
    let derived = LaunchBoundary::checked(&record, &spec).expect("derived boundary");
    record.launch = Some(derived.clone());
    assert_eq!(
        LaunchBoundary::checked(&record, &spec).expect("stored boundary"),
        derived
    );

    let edited = AgentSpec::acp("fixture", Command::new(&program, ["--different".into()]));
    assert!(matches!(
        LaunchBoundary::checked(&record, &edited),
        Err(Error::UnknownBoundary)
    ));

    record
        .launch
        .as_mut()
        .expect("stored boundary")
        .bootstrap_args
        .push("unexpected".into());
    assert!(matches!(
        LaunchBoundary::checked(&record, &spec),
        Err(Error::UnknownBoundary)
    ));
    let error = build(
        &spec,
        &program,
        &method(Arguments::ReplaceApplication, None),
        Some(&record),
    )
    .expect_err("altered metadata cannot authorize argument replacement");
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::UnknownBoundary)
    ));
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
