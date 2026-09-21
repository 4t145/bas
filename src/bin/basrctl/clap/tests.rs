use super::*;

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
