use super::*;

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
