use super::*;

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
