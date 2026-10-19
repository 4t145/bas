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
