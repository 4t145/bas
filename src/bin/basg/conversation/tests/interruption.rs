use super::*;

#[test]
fn interruption_seals_partial_content_and_deduplicates_following_failure() {
    let mut conversation = conversation();
    let turn = TurnId::new();
    apply(&mut conversation, 1, EventKind::TurnStarted { turn });
    apply(
        &mut conversation,
        2,
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("partial answer".into()),
        )),
    );
    apply(
        &mut conversation,
        3,
        EventKind::TurnInterrupted {
            turn,
            reason: InterruptionReason::BackendFailure {
                message: "backend lost".into(),
            },
        },
    );
    assert!(
        matches!(&conversation.items[0], Item::Message { message, streaming: false } if message.plain_text() == "partial answer")
    );
    assert!(conversation.last_reply.is_some());
    assert!(conversation.completed_reply.is_none());
    assert_eq!(conversation.session.status, SessionStatus::Idle);
    assert!(
        matches!(&conversation.items[1], Item::Notice { text, .. } if text.contains("Bas") && text.contains("backend lost"))
    );
    apply(
        &mut conversation,
        4,
        EventKind::Failed {
            message: "backend lost".into(),
        },
    );
    assert_eq!(conversation.items.len(), 2);
    assert_eq!(conversation.session.status, SessionStatus::Failed);
    apply(
        &mut conversation,
        5,
        EventKind::Failed {
            message: "different failure".into(),
        },
    );
    assert_eq!(conversation.items.len(), 3);
}

#[test]
fn native_error_uses_agent_message_not_generic_error_display() {
    let mut conversation = conversation();
    let turn = TurnId::new();
    apply(&mut conversation, 1, EventKind::TurnStarted { turn });
    apply(
        &mut conversation,
        2,
        EventKind::TurnFinished {
            turn,
            result: Err(acp::Error::new(-32000, "native rejection")),
        },
    );
    assert!(
        matches!(&conversation.items[0], Item::Notice { text, .. } if text.contains("native rejection") && !text.contains("Bas") && !text.contains("-32000"))
    );
}
