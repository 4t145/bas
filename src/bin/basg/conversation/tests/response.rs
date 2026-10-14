use super::*;

fn user(text: &str) -> EventKind {
    EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
        acp::ContentChunk::new(text.into()),
    ))
}

fn agent(text: &str) -> EventKind {
    EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
        acp::ContentChunk::new(text.into()),
    ))
}

fn thought() -> EventKind {
    EventKind::Acp(acp::SessionUpdate::AgentThoughtChunk(
        acp::ContentChunk::new("not copied".into()),
    ))
}

fn tool(id: &str) -> EventKind {
    EventKind::Acp(acp::SessionUpdate::ToolCall(ToolCall::new(
        id.to_owned(),
        "not copied",
    )))
}

fn apply(conversation: &mut Conversation, kinds: impl IntoIterator<Item = EventKind>) {
    for kind in kinds {
        conversation.apply(Event::new(
            conversation.session.id,
            Cursor::from_raw(conversation.cursor + 1),
            kind,
        ));
    }
}

#[test]
fn unanswered_second_turn_does_not_copy_the_previous_response() {
    for result in [
        Ok(acp::PromptResponse::new(StopReason::Cancelled)),
        Err(acp::Error::internal_error()),
    ] {
        let mut conversation = conversation();
        let first = TurnId::new();
        let second = TurnId::new();
        apply(
            &mut conversation,
            [
                EventKind::TurnStarted { turn: first },
                user("first"),
                agent("first answer"),
                EventKind::TurnFinished {
                    turn: first,
                    result: Ok(acp::PromptResponse::new(StopReason::EndTurn)),
                },
                EventKind::TurnStarted { turn: second },
                user("second"),
                thought(),
                tool("second-tool"),
                EventKind::TurnFinished {
                    turn: second,
                    result,
                },
            ],
        );
        let tail = conversation.response(5).expect("tail navigation");
        assert!(tail.text.is_empty());
        assert_eq!(tail.user_item, Some(2));
        assert!(tail.latest);
        let first = conversation.response(1).expect("first response");
        assert_eq!(first.text, "first answer");
        assert_eq!(first.user_item, Some(0));
        assert!(!first.latest);
    }
}

#[test]
fn response_groups_noncontiguous_messages_at_one_endpoint_and_survives_replay() {
    let mut conversation = conversation();
    let kinds = [
        agent("greeting"),
        user("first"),
        agent("**first**"),
        tool("read"),
        thought(),
        agent(" \n "),
        tool("verify"),
        agent("second\nline"),
        thought(),
        tool("tail"),
        user("next"),
    ];
    let events: Vec<_> = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            Event::new(
                conversation.session.id,
                Cursor::from_raw(index as u64 + 1),
                kind,
            )
        })
        .collect();
    for event in &events {
        conversation.apply(event.clone());
    }
    let mut replay = Conversation::new(conversation.session.clone());
    for event in events.iter().chain(&events) {
        replay.apply(event.clone());
    }
    for conversation in [&conversation, &replay] {
        let greeting = conversation.response(0).expect("initial greeting");
        assert_eq!(greeting.text, "greeting");
        assert_eq!(greeting.user_item, None);
        assert!(!greeting.latest);
        for index in 1..9 {
            assert!(conversation.response(index).is_none(), "item {index}");
        }
        let response = conversation.response(9).expect("trailing tool endpoint");
        assert_eq!(response.text, "**first**\n\nsecond\nline");
        assert_eq!(response.user_item, Some(1));
        assert!(!response.latest);
        assert!(conversation.response(10).is_none());
    }
}

#[test]
fn trailing_thought_keeps_response_controls_after_the_next_user_arrives() {
    let mut conversation = conversation();
    apply(
        &mut conversation,
        [user("first"), agent("answer"), thought()],
    );
    let response = conversation.response(2).expect("tail thought endpoint");
    assert_eq!(response.text, "answer");
    assert!(response.latest);
    apply(&mut conversation, [user("next")]);
    assert!(conversation.response(1).is_none());
    let response = conversation
        .response(2)
        .expect("historical thought endpoint");
    assert_eq!(response.text, "answer");
    assert_eq!(response.user_item, Some(0));
    assert!(!response.latest);
}
