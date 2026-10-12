use super::*;

#[test]
fn agent_management_preserves_existing_agents_command() {
    assert!(matches!(
        Args::try_parse_from(["basrctl", "agents"])
            .expect("existing command")
            .command,
        Command::Agents
    ));
    assert!(matches!(
        Args::try_parse_from(["basrctl", "agent", "catalog", "--refresh", "--json"])
            .expect("catalog")
            .command,
        Command::Agent(Agent::Catalog {
            refresh: true,
            json: true
        })
    ));
    assert!(
        matches!(Args::try_parse_from(["basrctl", "agent", "login", "codex-acp", "--method", "login"]).expect("login").command,
        Command::Agent(Agent::Login { id, method: Some(method) }) if id == "codex-acp" && method == "login")
    );
    for operation in ["install", "check", "login", "remove"] {
        assert!(
            Args::try_parse_from(["basrctl", "agent", operation]).is_err(),
            "{operation} needs an id"
        );
    }
}

#[test]
fn list_defaults_to_non_archived_and_all_is_explicit() {
    for (args, expected) in [
        (vec!["basrctl", "session", "list"], false),
        (vec!["basrctl", "session", "list", "--all"], true),
    ] {
        let parsed = Args::try_parse_from(args).expect("valid list command");
        assert!(
            matches!(parsed.command, Command::Session(Session::List { all }) if all == expected)
        );
    }
}

#[test]
fn archive_restore_and_remove_are_distinct_commands_requiring_an_id() {
    for command in ["archive", "restore", "remove"] {
        assert!(Args::try_parse_from(["basrctl", "session", command]).is_err());
        let parsed = Args::try_parse_from(["basrctl", "session", command, "session-id"])
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
fn remove_help_describes_permanent_not_recoverable_removal() {
    let help = Args::try_parse_from(["basrctl", "session", "remove", "--help"])
        .expect_err("help is returned instead of a command")
        .to_string();
    assert!(help.contains("Permanently stop"));
    assert!(help.contains("not recoverable archive"));
}

#[test]
fn prompt_submission_is_optional_and_retry_id_is_preserved() {
    let turn = basr::model::TurnId::new();
    let turn_text = turn.to_string();
    let parsed = Args::try_parse_from([
        "basrctl",
        "prompt",
        "session",
        "--submission",
        &turn_text,
        "hello",
        "world",
    ])
    .expect("explicit retry");
    assert!(matches!(
        parsed.command,
        Command::Prompt { submission: Some(id), text, .. }
            if id == turn && text == ["hello", "world"]
    ));
    let parsed =
        Args::try_parse_from(["basrctl", "prompt", "session", "hello"]).expect("new submission");
    assert!(matches!(
        parsed.command,
        Command::Prompt {
            submission: None,
            ..
        }
    ));
    assert!(
        Args::try_parse_from([
            "basrctl",
            "prompt",
            "session",
            "--submission",
            "not-a-turn",
            "hello",
        ])
        .is_err()
    );
}

#[test]
fn prompt_status_requires_session_and_turn() {
    let session = basr::model::SessionId::new();
    let turn = basr::model::TurnId::new();
    let parsed = Args::try_parse_from([
        "basrctl",
        "prompt-status",
        &session.to_string(),
        &turn.to_string(),
    ])
    .expect("read-only status command");
    assert!(matches!(parsed.command,
        Command::PromptStatus { session: id, turn: submission } if id == session && submission == turn
    ));
    assert!(Args::try_parse_from(["basrctl", "prompt-status", &session.to_string()]).is_err());
}
