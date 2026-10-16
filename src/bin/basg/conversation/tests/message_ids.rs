use super::*;

fn identified(role: Role, id: &str, text: &str) -> EventKind {
    let chunk = acp::ContentChunk::new(text.into()).message_id(acp::MessageId::from(id.to_owned()));
    EventKind::Acp(match role {
        Role::User => acp::SessionUpdate::UserMessageChunk(chunk),
        Role::Agent => acp::SessionUpdate::AgentMessageChunk(chunk),
        Role::Thought => acp::SessionUpdate::AgentThoughtChunk(chunk),
    })
}

fn texts(conversation: &Conversation) -> Vec<String> {
    conversation
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Message { message, .. } => Some(message.plain_text()),
            _ => None,
        })
        .collect()
}

#[test]
fn consecutive_messages_with_different_ids_remain_distinct() {
    let mut conversation = conversation();
    apply(&mut conversation, 1, identified(Role::Agent, "first", "A"));
    apply(&mut conversation, 2, identified(Role::Agent, "second", "B"));
    apply(&mut conversation, 3, identified(Role::Agent, "second", "C"));
    assert_eq!(texts(&conversation), ["A", "BC"]);
}

#[test]
fn interleaved_chunks_return_to_their_message_and_track_the_actual_reply() {
    let mut conversation = conversation();
    let turn = TurnId::new();
    apply(&mut conversation, 1, EventKind::TurnStarted { turn });
    apply(&mut conversation, 2, identified(Role::Agent, "a", "A"));
    apply(&mut conversation, 3, identified(Role::Agent, "b", "B"));
    apply(
        &mut conversation,
        4,
        EventKind::Acp(acp::SessionUpdate::ToolCall(ToolCall::new("tool", "Read"))),
    );
    apply(&mut conversation, 5, identified(Role::Agent, "a", "C"));
    assert_eq!(texts(&conversation), ["AC", "B"]);
    assert_eq!(conversation.last_reply.expect("reply").index, 0);
    apply(
        &mut conversation,
        6,
        EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(StopReason::EndTurn)),
        },
    );
    assert_eq!(conversation.completed_reply.expect("completed").index, 0);
}

#[test]
fn identity_is_scoped_by_role_and_session_and_supports_opaque_ids() {
    let mut first = conversation();
    let mut second = conversation();
    let id = "not-a-uuid/消息";
    for (index, role) in [Role::User, Role::Thought, Role::Agent]
        .into_iter()
        .enumerate()
    {
        apply(&mut first, index as u64 + 1, identified(role, id, "A"));
    }
    apply(&mut first, 4, identified(Role::Thought, id, "B"));
    apply(&mut second, 1, identified(Role::Agent, id, "C"));
    assert_eq!(texts(&first), ["A", "AB", "A"]);
    assert_eq!(texts(&second), ["C"]);
}

#[test]
fn idless_chunks_keep_legacy_grouping_without_joining_identified_messages() {
    let mut conversation = conversation();
    apply(&mut conversation, 1, chunk("A"));
    apply(&mut conversation, 2, chunk("B"));
    apply(&mut conversation, 3, identified(Role::Agent, "id", "C"));
    apply(&mut conversation, 4, chunk("D"));
    apply(&mut conversation, 5, chunk("E"));
    apply(&mut conversation, 6, identified(Role::Agent, "id", "F"));
    assert_eq!(texts(&conversation), ["AB", "CF", "DE"]);
}

#[test]
fn replay_preserves_identified_message_boundaries_and_deduplicates_cursors() {
    let mut live = conversation();
    let mut replay = Conversation::new(live.session.clone());
    for (index, kind) in [
        identified(Role::Agent, "a", "A"),
        identified(Role::Agent, "b", "B"),
        identified(Role::Agent, "a", "C"),
    ]
    .into_iter()
    .enumerate()
    {
        let event = Event::new(live.session.id, Cursor::from_raw(index as u64 + 1), kind);
        live.apply(event.clone());
        replay.apply(event.clone());
        replay.apply(event);
    }
    assert_eq!(texts(&live), ["AC", "B"]);
    assert_eq!(texts(&live), texts(&replay));
}
