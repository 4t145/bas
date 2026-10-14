use super::*;

fn encoded(bytes: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn metadata(value: serde_json::Value) -> acp2::Meta {
    serde_json::from_value(value).expect("metadata object")
}

#[test]
fn legacy_terminal_info_keeps_directory_and_command() {
    let mut terminals = HashMap::new();
    let meta = metadata(serde_json::json!({
        "terminal_info": {
            "terminal_id": "term",
            "cwd": "/work/project",
            "command": "cargo test"
        },
        "terminal_output": {"terminal_id": "term", "data": "first\nsecond"},
        "terminal_exit": {"terminal_id": "term", "exit_code": 0}
    }));
    apply(&mut terminals, Some(&meta));
    let terminal = &terminals["term"];
    assert_eq!(
        terminal.cwd.as_deref(),
        Some(std::path::Path::new("/work/project"))
    );
    assert_eq!(terminal.label.as_deref(), Some("cargo test"));
    assert_eq!(terminal.output, "first\nsecond");
    assert_eq!(terminal.exit.as_ref().expect("exit").exit_code, Some(0));

    let meta = metadata(serde_json::json!({
        "terminal_info": {"terminal_id": "term", "label": "Tests"}
    }));
    apply(&mut terminals, Some(&meta));
    let terminal = &terminals["term"];
    assert_eq!(terminal.label.as_deref(), Some("Tests"));
    assert!(terminal.cwd.is_some());
}

#[test]
fn native_snapshot_replaces_output_and_chunks_append() {
    let mut terminal = Terminal::default();
    terminal
        .push_chunk(&acp2::TerminalOutputChunk::new("term", encoded("stale")))
        .expect("valid chunk");
    terminal
        .apply_update(
            &acp2::TerminalUpdate::new("term")
                .command("cargo test")
                .cwd("/work")
                .output(acp2::TerminalOutput::new(encoded(
                    "\u{1b}[32mfirst\u{1b}[0m\r\nprogress: 1",
                )))
                .exit_status(acp2::TerminalExitStatus::new().exit_code(0)),
        )
        .expect("valid snapshot");
    terminal
        .push_chunk(&acp2::TerminalOutputChunk::new(
            "term",
            encoded("\rprogress: 2\r\n中文"),
        ))
        .expect("valid chunk");
    assert_eq!(terminal.output, "first\nprogress: 2\n中文");
    assert_eq!(terminal.label.as_deref(), Some("cargo test"));
    assert_eq!(terminal.cwd.as_deref(), Some(std::path::Path::new("/work")));
    assert_eq!(terminal.exit.as_ref().expect("exit").exit_code, Some(0));

    terminal
        .apply_update(&acp2::TerminalUpdate::new("term"))
        .expect("omitted values");
    assert_eq!(terminal.output, "first\nprogress: 2\n中文");
    assert_eq!(terminal.label.as_deref(), Some("cargo test"));
    assert!(terminal.exit.is_some());
}

#[test]
fn native_chunks_preserve_split_utf8_and_terminal_escape_sequences() {
    let mut terminal = Terminal::default();
    let bytes = "\u{1b}[32m中文\u{1b}[0m".as_bytes();
    for byte in bytes {
        terminal
            .push_chunk(&acp2::TerminalOutputChunk::new("term", encoded([*byte])))
            .expect("independently encoded chunk");
    }
    assert_eq!(terminal.output, "中文");
}

#[test]
fn null_fields_clear_terminal_state_and_reset_the_parser() {
    let mut terminal = Terminal::default();
    terminal
        .apply_update(
            &acp2::TerminalUpdate::new("term")
                .command("command")
                .cwd("/work")
                .output(
                    acp2::TerminalOutput::new(encoded("old\u{1b}["))
                        .meta(metadata(serde_json::json!({"snapshot": true}))),
                )
                .exit_status(acp2::TerminalExitStatus::new())
                .meta(metadata(serde_json::json!({"terminal": true}))),
        )
        .expect("initial state");
    // Even an empty exit object marks the terminal as exited.
    assert!(terminal.exit.is_some());
    let clear: acp2::TerminalUpdate = serde_json::from_value(serde_json::json!({
        "terminalId": "term", "command": null, "cwd": null,
        "output": null, "exitStatus": null, "_meta": null
    }))
    .expect("clear patch");
    terminal.apply_update(&clear).expect("valid clear");
    assert!(terminal.output.is_empty());
    assert!(terminal.cwd.is_none());
    assert!(terminal.label.is_none());
    assert!(terminal.exit.is_none());
    assert!(terminal.meta.is_none());
    assert!(terminal.output_meta.is_none());
    terminal
        .push_chunk(&acp2::TerminalOutputChunk::new("term", encoded("new")))
        .expect("new output");
    assert_eq!(terminal.output, "new");
}

#[test]
fn chunk_metadata_does_not_overwrite_terminal_or_snapshot_metadata() {
    let mut terminal = Terminal::default();
    let terminal_meta = metadata(serde_json::json!({"terminal": true}));
    let snapshot_meta = metadata(serde_json::json!({"snapshot": true}));
    terminal
        .apply_update(
            &acp2::TerminalUpdate::new("term")
                .output(acp2::TerminalOutput::new(encoded("a")).meta(snapshot_meta.clone()))
                .meta(terminal_meta.clone()),
        )
        .expect("valid update");
    terminal
        .push_chunk(
            &acp2::TerminalOutputChunk::new("term", encoded("b"))
                .meta(metadata(serde_json::json!({"chunk": true}))),
        )
        .expect("valid chunk");
    assert_eq!(terminal.output, "ab");
    assert_eq!(terminal.meta.as_ref(), Some(&terminal_meta));
    assert_eq!(terminal.output_meta.as_ref(), Some(&snapshot_meta));
}

#[test]
fn invalid_base64_reports_an_error_without_mutating_terminal_state() {
    let mut terminal = Terminal::default();
    terminal
        .apply_update(
            &acp2::TerminalUpdate::new("term")
                .command("original")
                .output(acp2::TerminalOutput::new(encoded("kept"))),
        )
        .expect("initial state");
    assert!(
        terminal
            .apply_update(
                &acp2::TerminalUpdate::new("term")
                    .command("changed")
                    .output(acp2::TerminalOutput::new("!invalid")),
            )
            .is_err()
    );
    assert_eq!(terminal.output, "kept");
    assert_eq!(terminal.label.as_deref(), Some("original"));
    assert!(
        terminal
            .push_chunk(&acp2::TerminalOutputChunk::new("term", "!invalid"))
            .is_err()
    );
    assert_eq!(terminal.output, "kept");
}
