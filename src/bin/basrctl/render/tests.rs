use super::*;

#[test]
fn native_chunks_form_readable_lines_without_duplicate_commits() {
    let session = model::SessionId::new();
    let turn = model::TurnId::new();
    let updates = [
        EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
            acp::ContentChunk::new("hello".into()),
        )),
        EventKind::Acp(acp::SessionUpdate::AgentThoughtChunk(
            acp::ContentChunk::new("thinking".into()),
        )),
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("BASR_".into()),
        )),
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("PI_OK".into()),
        )),
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        },
    ];
    let mut timeline = Timeline::default();
    let mut output = Vec::new();
    for (cursor, kind) in updates.into_iter().enumerate() {
        timeline
            .write_event(
                &model::Event::new(session, model::Cursor::from_raw(cursor as u64 + 1), kind),
                &mut output,
            )
            .expect("render");
    }
    let output = String::from_utf8(output).expect("UTF-8");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].ends_with("user hello"));
    assert!(lines[1].ends_with("thought thinking"));
    assert!(lines[2].ends_with("agent BASR_PI_OK"));
    assert!(lines[3].contains("finished (end_turn)"));
}
