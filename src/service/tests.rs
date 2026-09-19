use super::{Error, pb};
use crate::{acp, model::*};

fn event(kind: EventKind) -> Event {
    Event::new(SessionId::new(), Cursor::from_raw(17), kind)
}

#[test]
fn native_acp_updates_survive_the_protobuf_boundary_without_loss() {
    use prost::Message;
    let updates: Vec<acp::SessionUpdate> =
        serde_json::from_str(include_str!("assets/updates.json")).expect("ACP fixtures");
    assert_eq!(updates.len(), 14);
    for update in updates {
        let original = event(EventKind::Acp(update.clone()));
        let bytes = pb::Event::from(&original).encode_to_vec();
        let decoded = Event::try_from(pb::Event::decode(bytes.as_slice()).expect("protobuf"))
            .expect("ACP event");
        assert_eq!(decoded, original);
        let payload = pb::AcpPayload::from(&update);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&payload.json).expect("JSON"),
            serde_json::to_value(update).expect("SDK JSON")
        );
    }
}

#[test]
fn prompt_content_and_extension_metadata_use_the_sdk_schema() {
    let updates: Vec<acp::SessionUpdate> =
        serde_json::from_str(include_str!("assets/updates.json")).expect("ACP fixtures");
    let content = updates
        .into_iter()
        .filter_map(|update| match update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => Some(chunk.content),
            _ => None,
        })
        .collect();
    let request = acp::PromptRequest::new(SessionId::new().to_string(), content).meta(
        serde_json::from_value::<acp::Meta>(serde_json::json!({"large":9007199254740993_u64}))
            .expect("metadata"),
    );
    let decoded: acp::PromptRequest = pb::AcpPayload::from(&request).decode().expect("prompt");
    assert_eq!(decoded, request);
    assert_eq!(decoded.prompt.len(), 5);
}

#[test]
fn lifecycle_and_complete_prompt_responses_round_trip() {
    for kind in [
        EventKind::TurnStarted {
            turn: TurnId::new(),
        },
        EventKind::TurnFinished {
            turn: TurnId::new(),
            result: Ok(
                acp::PromptResponse::new(acp::StopReason::MaxTurnRequests).meta(
                    serde_json::from_value::<acp::Meta>(serde_json::json!({"cost":1.2}))
                        .expect("metadata"),
                ),
            ),
        },
        EventKind::TurnFinished {
            turn: TurnId::new(),
            result: Err("connection lost".into()),
        },
        EventKind::TitleChanged {
            title: "renamed".into(),
        },
        EventKind::StatusChanged {
            status: SessionStatus::Working,
        },
        EventKind::Failed {
            message: "backend failed".into(),
        },
    ] {
        let original = event(kind);
        assert_eq!(
            Event::try_from(pb::Event::from(&original)).expect("event"),
            original
        );
    }
}

#[test]
fn malformed_envelopes_and_acp_payloads_are_rejected() {
    assert!(matches!(
        pb::AcpPayload {
            json: b"broken JSON".to_vec()
        }
        .decode::<acp::PromptRequest>(),
        Err(Error::Json(_))
    ));
    assert!(matches!(
        pb::AcpPayload {
            json: b"{}".to_vec()
        }
        .decode::<acp::PromptRequest>(),
        Err(Error::Json(_))
    ));
    let mut wire = pb::Event::from(&event(EventKind::TurnStarted {
        turn: TurnId::new(),
    }));
    wire.session = "not-a-session".into();
    assert!(matches!(Event::try_from(wire), Err(Error::Id(_))));
    let wire = pb::Event {
        session: SessionId::new().to_string(),
        kind: None,
        ..Default::default()
    };
    assert!(matches!(
        Event::try_from(wire),
        Err(Error::MissingEventKind)
    ));
    let wire = pb::Event {
        session: SessionId::new().to_string(),
        kind: Some(pb::event::Kind::TurnFinished(pb::TurnFinished {
            turn: TurnId::new().to_string(),
            outcome: None,
        })),
        ..Default::default()
    };
    assert!(matches!(Event::try_from(wire), Err(Error::MissingOutcome)));
}

#[test]
fn session_capabilities_modes_and_options_round_trip() {
    let mut session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    session.initialization = Some(
        acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .agent_capabilities(
                acp::AgentCapabilities::new().prompt_capabilities(
                    acp::PromptCapabilities::new()
                        .image(true)
                        .embedded_context(true),
                ),
            ),
    );
    session.acp = Some(
        acp::NewSessionResponse::new(session.id.to_string()).config_options(vec![
            acp::SessionConfigOption::boolean("enabled", "Enabled", true),
        ]),
    );
    session.available_commands = Some(acp::AvailableCommandsUpdate::new(vec![
        acp::AvailableCommand::new("help", "Help"),
    ]));
    assert_eq!(
        SessionMeta::try_from(pb::Session::from(&session)).expect("metadata"),
        session
    );
}

#[test]
fn missing_owner_and_unknown_lifecycle_status_are_rejected() {
    let session = pb::Session {
        id: SessionId::new().to_string(),
        status: pb::SessionStatus::Idle as i32,
        ..Default::default()
    };
    assert!(matches!(
        SessionMeta::try_from(session.clone()),
        Err(Error::MissingOwner)
    ));
    let session = pb::Session {
        owner: Some(pb::Owner::default()),
        status: 999,
        ..session
    };
    assert!(matches!(
        SessionMeta::try_from(session),
        Err(Error::Unknown {
            field: "session status",
            ..
        })
    ));
}
