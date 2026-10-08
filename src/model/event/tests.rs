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
fn legacy_flattened_updates_including_duplicate_native_kind_remain_readable() {
    let id = SessionId::new();
    let encoded = format!(
        r#"{{"session":"{id}","cursor":1,"at":0,"kind":"acp","sessionUpdate":"tool_call","toolCallId":"command","title":"printf result","kind":"execute"}}"#
    );
    let event: Event = serde_json::from_str(&encoded).expect("legacy duplicate discriminator");
    let EventKind::Acp(acp::SessionUpdate::ToolCall(tool)) = event.kind else {
        panic!("native tool");
    };
    assert_eq!(tool.kind, acp::ToolKind::Execute);
    let encoded = format!(
        r#"{{"session":"{id}","cursor":2,"at":0,"kind":"acp","sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"legacy message"}}}}"#
    );
    assert!(matches!(
        serde_json::from_str::<Event>(&encoded)
            .expect("legacy content")
            .kind,
        EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(_))
    ));
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
