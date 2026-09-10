use std::str::FromStr;

use super::{ContentBlock, Cursor, DeltaBlock, Role, SessionId, ToolKind};

#[test]
fn parses_and_prints_session_ids() {
    let id = SessionId::new();
    let text = id.to_string();

    assert_eq!(SessionId::from_str(&text).unwrap(), id);
    assert!(SessionId::from_str("not-a-uuid").is_err());
    assert!(
        SessionId::from_str("not-a-uuid")
            .unwrap_err()
            .to_string()
            .contains("session id")
    );
}

#[test]
fn cursors_increase_by_one() {
    assert_eq!(Cursor::START.next().raw(), 1);
    assert_eq!(Cursor::START.next().next().raw(), 2);
    assert_eq!(Cursor::from_raw(41).raw(), 41);
}

#[test]
fn deltas_accumulate_into_the_open_block() {
    let mut blocks = Vec::new();

    DeltaBlock::Text {
        text: "hel".to_owned(),
    }
    .apply(&mut blocks);
    DeltaBlock::Text {
        text: "lo".to_owned(),
    }
    .apply(&mut blocks);
    DeltaBlock::Thinking {
        text: "hmm".to_owned(),
    }
    .apply(&mut blocks);
    DeltaBlock::Text {
        text: "!".to_owned(),
    }
    .apply(&mut blocks);

    assert_eq!(
        blocks,
        vec![
            ContentBlock::text("hello"),
            ContentBlock::Thinking(super::Text::new("hmm")),
            ContentBlock::text("!"),
        ]
    );
}

#[test]
fn plain_text_joins_text_blocks_only() {
    let message = super::Message::new(
        Role::Agent,
        vec![
            ContentBlock::text("done "),
            ContentBlock::ToolCall(super::ToolCall::new(
                super::ToolCallId::new("call-1"),
                "read a file",
            )),
            ContentBlock::text("ok"),
        ],
    );

    assert_eq!(message.plain_text(), "done ok");
}

#[test]
fn tool_calls_start_pending() {
    let call = super::ToolCall::new(super::ToolCallId::new("call-1"), "grep");

    assert_eq!(call.kind, ToolKind::Other);
    assert_eq!(call.status, super::ToolCallStatus::Pending);
}

#[test]
fn only_committed_events_are_replayable() {
    let session = SessionId::new();
    let delta = super::Event::new(
        session,
        Cursor::START,
        super::EventKind::Delta(super::Delta {
            message: super::MessageId::new(),
            role: Role::Agent,
            block: DeltaBlock::Text {
                text: "hi".to_owned(),
            },
        }),
    );
    let message = super::Event::new(
        session,
        Cursor::START.next(),
        super::EventKind::Message(super::Message::text(Role::Agent, "hi")),
    );

    assert!(!delta.is_replayable());
    assert!(message.is_replayable());
    assert_eq!(message.message().unwrap().plain_text(), "hi");
}
