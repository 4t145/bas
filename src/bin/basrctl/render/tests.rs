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

#[test]
fn every_timeline_field_crosses_the_safe_output_boundary() {
    let malicious = "\x1b]52;c;clipboard\x07\r\u{009b}2J中文";
    let updates = [
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("\x1b".into()),
        )),
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("]52;c;clipboard\x07".into()),
        )),
        EventKind::Acp(acp::SessionUpdate::ToolCall(acp::ToolCall::new(
            malicious, malicious,
        ))),
        EventKind::Acp(acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(malicious, acp::ToolCallUpdateFields::default()),
        )),
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                malicious, malicious,
            ))),
        )),
        EventKind::Failed {
            message: malicious.into(),
        },
        EventKind::TurnFinished {
            turn: model::TurnId::new(),
            result: Err(malicious.into()),
        },
        EventKind::TitleChanged {
            title: malicious.into(),
        },
    ];
    let mut timeline = Timeline::default();
    let mut output = Vec::new();
    for kind in updates {
        timeline
            .write_event(
                &model::Event::new(model::SessionId::new(), model::Cursor::from_raw(1), kind),
                &mut output,
            )
            .expect("valid timeline events render into an in-memory buffer");
    }
    let output = String::from_utf8(output).expect("safe rendering emits only UTF-8");
    assert!(
        !output
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    );
    assert!(output.contains("\\u001b]52;c;clipboard\\u0007"));
    assert!(output.contains("中文"));
}
