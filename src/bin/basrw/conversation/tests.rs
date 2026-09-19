use super::*;

fn conversation() -> Conversation {
    Conversation::new(SessionMeta::new(
        SessionId::new(),
        "embedded".into(),
        Owner::current(),
    ))
}
fn apply(conversation: &mut Conversation, cursor: u64, kind: EventKind) {
    conversation.apply(Event::new(
        conversation.session.id,
        Cursor::from_raw(cursor),
        kind,
    ));
}
fn chunk(text: &str) -> EventKind {
    EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
        acp::ContentChunk::new(text.into()),
    ))
}

#[test]
fn live_and_replayed_acp_chunks_produce_the_same_messages() {
    let mut live = conversation();
    let turn = TurnId::new();
    let events = vec![
        EventKind::TurnStarted { turn },
        EventKind::Acp(acp::SessionUpdate::AgentThoughtChunk(
            acp::ContentChunk::new("思考".into()),
        )),
        chunk("你好"),
        chunk("，世界"),
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(StopReason::EndTurn)),
        },
    ];
    let mut replay = Conversation::new(live.session.clone());
    for (index, event) in events.into_iter().enumerate() {
        let event = Event::new(live.session.id, Cursor::from_raw(index as u64 + 1), event);
        live.apply(event.clone());
        replay.apply(event);
    }
    for conversation in [&live, &replay] {
        let reply = conversation
            .completed_reply
            .expect("completed final response");
        assert_eq!(reply.index, 1);
        assert_eq!(reply.cursor, 5);
        assert_eq!(conversation.items.len(), 2);
        assert!(
            matches!(&conversation.items[0], Item::Message { message, streaming: false } if message.role == Role::Thought)
        );
        assert!(
            matches!(&conversation.items[1], Item::Message { message, streaming: false } if message.plain_text() == "你好，世界")
        );
    }
}

#[test]
fn completion_selects_the_final_reply_and_never_reuses_a_previous_turn() {
    let mut conversation = conversation();
    let turn = TurnId::new();
    apply(&mut conversation, 1, EventKind::TurnStarted { turn });
    apply(&mut conversation, 2, chunk("先检查文件"));
    apply(
        &mut conversation,
        3,
        EventKind::Acp(acp::SessionUpdate::ToolCall(ToolCall::new("read", "read"))),
    );
    apply(&mut conversation, 4, chunk("最终答复"));
    assert!(conversation.completed_reply.is_none());
    apply(
        &mut conversation,
        5,
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(StopReason::EndTurn)),
        },
    );
    let reply = conversation.completed_reply.expect("final answer");
    assert_eq!(reply.index, 2);
    assert_eq!(reply.cursor, 5);
    apply(
        &mut conversation,
        6,
        EventKind::TurnStarted {
            turn: TurnId::new(),
        },
    );
    assert!(conversation.completed_reply.is_none());
    apply(
        &mut conversation,
        7,
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(StopReason::Cancelled)),
        },
    );
    assert!(conversation.completed_reply.is_none());
}

#[test]
fn replayed_events_and_events_from_other_sessions_are_ignored() {
    let mut conversation = conversation();
    let event = Event::new(
        conversation.session.id,
        Cursor::from_raw(10),
        chunk("hello"),
    );
    conversation.apply(event.clone());
    conversation.apply(event);
    conversation.apply(Event::new(
        SessionId::new(),
        Cursor::from_raw(11),
        EventKind::Failed {
            message: "wrong session".into(),
        },
    ));
    assert_eq!(conversation.cursor, 10);
    assert_eq!(conversation.items.len(), 1);
    assert_ne!(conversation.session.status, SessionStatus::Failed);
}

#[test]
fn partial_tool_updates_reuse_acp_merging_and_preserve_content_locations_and_input() {
    let mut conversation = conversation();
    let tool = ToolCall::new("read-1", "Read Cargo.toml")
        .raw_input(serde_json::json!({"path":"Cargo.toml"}))
        .locations(vec![acp::ToolCallLocation::new("Cargo.toml").line(12)])
        .content(vec![acp::ToolCallContent::Content(acp::Content::new(
            "output",
        ))]);
    apply(
        &mut conversation,
        1,
        EventKind::Acp(acp::SessionUpdate::ToolCall(tool.clone())),
    );
    apply(
        &mut conversation,
        2,
        EventKind::Acp(acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                "read-1",
                acp::ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            ),
        )),
    );
    let Item::Tool(actual) = &conversation.items[0] else {
        panic!("tool expected")
    };
    assert_eq!(actual.raw_input, tool.raw_input);
    assert_eq!(actual.locations, tool.locations);
    assert_eq!(actual.content, tool.content);
    assert_eq!(actual.status, ToolCallStatus::Completed);
    assert_eq!(conversation.items.len(), 1);
}

#[test]
fn images_files_and_configuration_survive_replay() {
    let mut conversation = conversation();
    let updates: Vec<acp::SessionUpdate> =
        serde_json::from_str(include_str!("../../../service/assets/updates.json"))
            .expect("ACP fixtures");
    for (index, update) in updates.into_iter().enumerate() {
        apply(&mut conversation, index as u64 + 1, EventKind::Acp(update));
    }
    let Item::Message { message, .. } = &conversation.items[0] else {
        panic!("message")
    };
    assert_eq!(message.content.len(), 5);
    assert!(matches!(message.content[0], ContentBlock::Image(_)));
    assert!(matches!(message.content[1], ContentBlock::Resource(_)));
    assert_eq!(
        conversation
            .session
            .acp
            .expect("state")
            .config_options
            .expect("options")
            .len(),
        2
    );
    assert_eq!(conversation.commands.len(), 1);
    assert!(conversation.plan.is_some());
    assert!(conversation.usage.is_some());
}

#[test]
fn cancellation_and_failure_update_status_and_seal_partial_output() {
    let mut conversation = conversation();
    let turn = TurnId::new();
    apply(&mut conversation, 1, EventKind::TurnStarted { turn });
    apply(&mut conversation, 2, chunk("partial"));
    apply(
        &mut conversation,
        3,
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(StopReason::Cancelled)),
        },
    );
    assert_eq!(conversation.session.status, SessionStatus::Idle);
    assert!(matches!(
        &conversation.items[0],
        Item::Message {
            streaming: false,
            ..
        }
    ));
    assert!(matches!(&conversation.items[1], Item::Notice { text, .. } if text.contains("停止")));
    apply(
        &mut conversation,
        4,
        EventKind::Failed {
            message: "connection lost".into(),
        },
    );
    assert_eq!(conversation.session.status, SessionStatus::Failed);
}

#[test]
fn tool_completion_stays_between_the_surrounding_answer_chunks() {
    let mut conversation = conversation();
    apply(&mut conversation, 1, chunk("先查看文件。"));
    apply(
        &mut conversation,
        2,
        EventKind::Acp(acp::SessionUpdate::ToolCall(
            ToolCall::new("read", "读取文件").status(ToolCallStatus::InProgress),
        )),
    );
    assert!(matches!(
        &conversation.items[0],
        Item::Message {
            streaming: false,
            ..
        }
    ));
    apply(&mut conversation, 3, chunk("文件中有一个问题。"));
    apply(
        &mut conversation,
        4,
        EventKind::Acp(acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                "read",
                acp::ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            ),
        )),
    );
    apply(&mut conversation, 5, chunk("可以这样修复。"));
    assert_eq!(conversation.items.len(), 3);
    assert!(
        matches!(&conversation.items[0], Item::Message { message, .. } if message.plain_text() == "先查看文件。")
    );
    assert!(
        matches!(&conversation.items[1], Item::Tool(tool) if tool.status == ToolCallStatus::Completed)
    );
    assert!(
        matches!(&conversation.items[2], Item::Message { message, .. } if message.plain_text() == "文件中有一个问题。可以这样修复。")
    );
}

#[test]
fn terminal_output_stream_survives_status_updates_and_replay_without_ansi_artifacts() {
    let mut conversation = conversation();
    let events = [
        serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"command","title":"fastfetch","kind":"execute","content":[{"type":"terminal","terminalId":"term"}],"_meta":{"terminal_info":{"terminal_id":"term","cwd":"/tmp"}}}),
        serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"command","_meta":{"terminal_output":{"terminal_id":"term","data":"\u{1b}[32mLinux\u{1b}[0m\nprogress: 1"}}}),
        serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"command","_meta":{"terminal_output":{"terminal_id":"term","data":"\rprogress: 2\n中文输出"}}}),
        serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"command","status":"completed","_meta":{"terminal_exit":{"terminal_id":"term","exit_code":0,"signal":null}}}),
    ];
    for (index, value) in events.into_iter().enumerate() {
        let event = Event::new(
            conversation.session.id,
            Cursor::from_raw(index as u64 + 1),
            EventKind::Acp(serde_json::from_value(value).expect("ACP update")),
        );
        conversation.apply(event.clone());
        conversation.apply(event); // Resume must not append duplicate terminal bytes.
    }
    let terminal = &conversation.terminals["term"];
    assert_eq!(terminal.output, "Linux\nprogress: 2\n中文输出");
    assert_eq!(terminal.exit.as_ref().expect("exit").exit_code, Some(0));
    assert_eq!(conversation.items.len(), 1);
}
