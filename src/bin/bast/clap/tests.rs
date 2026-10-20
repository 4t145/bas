use super::*;
use clap::{CommandFactory, FromArgMatches};

#[test]
fn tls_credentials_require_an_explicit_tls_target_and_complete_files() {
    for target in ["local", "tcp://server", "mtls://server:7338"] {
        let args = Args::try_parse_from([
            "bast",
            "--server",
            target,
            "--tls-ca",
            "/certs/ca.pem",
            "--tls-certificate",
            "/certs/client.pem",
            "--tls-key",
            "/certs/client.key",
            "--tls-server-name",
            "expected.example",
            "agents",
        ])
        .expect("syntax");
        assert_eq!(
            args.connection_options().is_ok(),
            target.starts_with("mtls://")
        );
    }
    let incomplete = Args::try_parse_from(["bast", "--server", "mtls://server:7338", "agents"])
        .expect("endpoint syntax");
    assert!(incomplete.connection_options().is_err());
    assert!(
        Args::try_parse_from([
            "bast",
            "--server",
            "mtls://server:7338",
            "--tls-key",
            "/certs/client.key",
            "agents",
        ])
        .is_err()
    );
}

#[test]
fn server_defaults_to_local_and_accepts_explicit_transports() {
    let parse = |args: Vec<&str>| {
        let matches = Args::command()
            .mut_arg("server", |arg| arg.env(None::<&str>))
            .try_get_matches_from(args)
            .expect("valid daemon target");
        Args::from_arg_matches(&matches)
            .expect("typed arguments")
            .server
    };
    assert_eq!(parse(vec!["bast", "agents"]), Target::Local);
    for (value, expected) in [
        ("local", Target::Local),
        ("unix:///tmp/bas.sock", Target::Unix("/tmp/bas.sock".into())),
        ("pipe://bas-test", Target::Pipe("bas-test".into())),
        (
            "tcp://127.0.0.1",
            Target::Tcp("127.0.0.1".parse().expect("TCP")),
        ),
        ("[::1]", Target::Tcp("[::1]".parse().expect("IPv6"))),
    ] {
        assert_eq!(parse(vec!["bast", "--server", value, "agents"]), expected);
        assert_eq!(parse(vec!["bast", "agents", "--server", value]), expected);
    }
}

#[test]
fn archive_restore_and_remove_are_distinct_commands_requiring_an_id() {
    for command in ["archive", "restore", "remove"] {
        assert!(Args::try_parse_from(["bast", "session", command]).is_err());
        let parsed = Args::try_parse_from(["bast", "session", command, "session-id"])
            .expect("valid session command");
        match (command, parsed.command) {
            ("archive", Command::Session(Session::Archive { session }))
            | ("restore", Command::Session(Session::Restore { session }))
            | ("remove", Command::Session(Session::Remove { session })) => {
                assert_eq!(session, "session-id");
            }
            other => panic!("incorrect command mapping: {other:?}"),
        }
    }
}

#[test]
fn prompt_status_requires_session_and_turn() {
    let session = bas::model::SessionId::new();
    let turn = bas::model::TurnId::new();
    let parsed = Args::try_parse_from([
        "bast",
        "prompt-status",
        &session.to_string(),
        &turn.to_string(),
    ])
    .expect("read-only status command");
    assert!(matches!(parsed.command,
        Command::PromptStatus { session: id, turn: submission } if id == session && submission == turn
    ));
    assert!(Args::try_parse_from(["bast", "prompt-status", &session.to_string()]).is_err());
}
