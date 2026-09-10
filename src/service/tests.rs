use crate::model::{
    ContentBlock, Cursor, Event, EventKind, Message, MessageId, Role, SessionId, StopReason,
    Timestamp, ToolCall, ToolCallId, ToolCallStatus, TurnId,
};

use super::{convert::Error, pb, status_label};

fn event(kind: EventKind) -> Event {
    Event {
        session: SessionId::new(),
        cursor: Cursor::from_raw(7),
        at: Timestamp::from_millis(1_700_000_000_000),
        kind,
    }
}

#[test]
fn events_survive_a_wire_round_trip() {
    let original = event(EventKind::Message(Message::with_id(
        MessageId::new(),
        Role::Agent,
        vec![
            ContentBlock::text("hello"),
            ContentBlock::ToolCall(ToolCall {
                id: ToolCallId::new("call-1"),
                title: "read a file".to_owned(),
                kind: crate::model::ToolKind::Read,
                status: ToolCallStatus::Completed,
                input: Some(serde_json::json!({"path": "/tmp/x"})),
                output: None,
                locations: vec!["/tmp/x".to_owned()],
            }),
        ],
    )));

    let decoded = Event::try_from(pb::Event::from(&original)).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn turn_lifecycle_events_survive_a_wire_round_trip() {
    let turn = TurnId::new();
    for kind in [
        EventKind::TurnStarted { turn },
        EventKind::TurnFinished {
            turn,
            stop: StopReason::Cancelled,
        },
        EventKind::TitleChanged {
            title: "renamed".to_owned(),
        },
        EventKind::StatusChanged {
            status: crate::model::SessionStatus::Working,
        },
    ] {
        let original = event(kind);
        let decoded = Event::try_from(pb::Event::from(&original)).unwrap();

        assert_eq!(decoded, original);
    }
}

#[test]
fn deltas_survive_a_wire_round_trip() {
    let original = event(EventKind::Delta(crate::model::Delta {
        message: MessageId::new(),
        role: Role::Agent,
        block: crate::model::DeltaBlock::Thinking {
            text: "considering".to_owned(),
        },
    }));

    let decoded = Event::try_from(pb::Event::from(&original)).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn malformed_wire_values_are_rejected() {
    let error = Event::try_from(pb::Event {
        session: "not-a-session".to_owned(),
        cursor: 0,
        at: 0,
        kind: Some(pb::event::Kind::TurnStarted(pb::TurnStarted {
            turn: TurnId::new().to_string(),
        })),
    })
    .unwrap_err();

    assert!(matches!(error, Error::Id(_)));

    let error = Event::try_from(pb::Event {
        session: SessionId::new().to_string(),
        cursor: 0,
        at: 0,
        kind: None,
    })
    .unwrap_err();

    assert!(matches!(error, Error::MissingEventKind));
}

#[test]
fn status_labels_are_stable() {
    assert_eq!(
        status_label(crate::model::SessionStatus::Working),
        "working"
    );
    assert_eq!(status_label(crate::model::SessionStatus::Failed), "failed");
}
