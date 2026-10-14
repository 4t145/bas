use super::*;

fn metadata(archived: bool) -> pb::Session {
    let mut session = model::SessionMeta::new(
        model::SessionId::new(),
        "test-agent".into(),
        model::Owner {
            user: "user".into(),
            host: "host".into(),
        },
    );
    session.archived = archived;
    session.title = Some("中文\x1b]52;c;clipboard\x07".into());
    pb::Session::try_from(&model::SessionSnapshot::from(session)).expect("valid fixture metadata")
}

#[test]
fn list_marks_only_archived_sessions_and_escapes_titles() {
    let mut output = Vec::new();
    write_sessions(&[metadata(false), metadata(true)], &mut output).expect("render list");
    let output = String::from_utf8(output).expect("UTF-8 output");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(!lines[0].contains("[archived]"));
    assert!(lines[1].ends_with("[archived]"));
    assert!(output.contains("中文\\u001b]52;c;clipboard\\u0007"));
    assert!(!output.contains('\x1b'));
}

#[test]
fn show_includes_explicit_archive_state() {
    for archived in [false, true] {
        let mut output = Vec::new();
        write_session(&metadata(archived), &mut output).expect("render metadata");
        let output = String::from_utf8(output).expect("UTF-8 output");
        assert!(
            output
                .lines()
                .any(|line| line == format!("archived  {archived}"))
        );
        assert!(!output.contains('\x1b'));
    }
}

#[test]
fn archive_events_end_chunks_and_render_both_states() {
    let mut timeline = Timeline::default();
    let mut output = Vec::new();
    let session = model::SessionId::new();
    for kind in [
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("text".into()),
        )),
        EventKind::ArchiveChanged { archived: true },
        EventKind::ArchiveChanged { archived: false },
    ] {
        timeline
            .write_event(
                &model::Event::new(session, model::Cursor::from_raw(1), kind),
                &mut output,
            )
            .expect("render event");
    }
    let output = String::from_utf8(output).expect("UTF-8 output");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].ends_with("agent text"));
    assert!(lines[1].ends_with("---- archived true"));
    assert!(lines[2].ends_with("---- archived false"));
}
