use super::*;

#[test]
fn native_tool_kind_roundtrips_without_colliding_with_the_event_discriminator() {
    let event = Event::new(
        SessionId::new(),
        Cursor::START.next(),
        EventKind::Acp(acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("command", "printf result").kind(acp::ToolKind::Execute),
        )),
    );
    let encoded = serde_json::to_vec(&event).expect("stored event");
    let value: serde_json::Value = serde_json::from_slice(&encoded).expect("event JSON");
    assert_eq!(value["kind"], "acp");
    assert_eq!(value["update"]["kind"], "execute");
    assert_eq!(
        serde_json::from_slice::<Event>(&encoded).expect("restore native tool"),
        event
    );
}

#[test]
fn ambiguous_flattened_updates_and_untyped_turn_errors_are_rejected() {
    let id = SessionId::new();
    let turn = TurnId::new();
    for encoded in [
        format!(
            r#"{{"session":"{id}","cursor":1,"at":0,"kind":"acp","sessionUpdate":"tool_call","toolCallId":"command","title":"printf result","kind":"execute"}}"#
        ),
        format!(
            r#"{{"session":"{id}","cursor":2,"at":0,"kind":"acp","sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"unwrapped message"}}}}"#
        ),
        format!(
            r#"{{"session":"{id}","cursor":3,"at":0,"kind":"turn_finished","turn":"{turn}","result":{{"Err":"unclassified failure"}}}}"#
        ),
    ] {
        assert!(serde_json::from_str::<Event>(&encoded).is_err());
    }
}

#[test]
fn native_prompt_errors_and_runtime_interruptions_have_distinct_storage_shapes() {
    let turn = TurnId::new();
    let native = acp::Error::new(-32000, "Sign in required").data(serde_json::json!({
        "reason": "expired",
        "counter": 9_007_199_254_740_993_u64
    }));
    let outcomes = [
        EventKind::TurnFinished {
            turn,
            result: Err(native.clone()),
        },
        EventKind::TurnInterrupted {
            turn,
            reason: InterruptionReason::DaemonRestarted,
        },
        EventKind::TurnInterrupted {
            turn,
            reason: InterruptionReason::DispatchFailure {
                message: "backend queue closed".into(),
            },
        },
    ];
    for kind in outcomes {
        let event = Event::new(SessionId::new(), Cursor::START.next(), kind);
        let json = serde_json::to_value(&event).expect("stored event");
        match &event.kind {
            EventKind::TurnFinished { .. } => {
                assert_eq!(json["kind"], "turn_finished");
                assert_eq!(
                    json["result"]["Err"],
                    serde_json::to_value(&native).expect("native error")
                );
                assert!(json.get("reason").is_none());
            }
            EventKind::TurnInterrupted { .. } => {
                assert_eq!(json["kind"], "turn_interrupted");
                assert!(json.get("result").is_none());
            }
            _ => unreachable!("only prompt results and interruptions in fixture"),
        }
        assert_eq!(
            serde_json::from_value::<Event>(json).expect("read event"),
            event
        );
    }
}

#[test]
fn lifecycle_events_keep_their_existing_storage_shape() {
    let event = Event::new(
        SessionId::new(),
        Cursor::START.next(),
        EventKind::TitleChanged {
            title: "thread".into(),
        },
    );
    let encoded = serde_json::to_vec(&event).expect("stored lifecycle");
    let value: serde_json::Value = serde_json::from_slice(&encoded).expect("JSON");
    assert_eq!(value["kind"], "title_changed");
    assert_eq!(value["title"], "thread");
    assert_eq!(
        serde_json::from_slice::<Event>(&encoded).expect("restore lifecycle"),
        event
    );
}
