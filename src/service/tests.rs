use super::{Error, pb};
use crate::{acp, model::*};

mod permissions;

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
        EventKind::ArchiveChanged { archived: true },
        EventKind::ArchiveChanged { archived: false },
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
    session.revision = Cursor::from_raw(123);
    session.active_turn = Some(TurnId::new());
    session.archived = true;
    session.last_reply = Cursor::from_raw(98);
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
    session.initialization_raw = session
        .initialization
        .as_ref()
        .map(|initialization| serde_json::to_value(initialization).expect("initialization JSON"));
    session.acp = Some(
        acp::NewSessionResponse::new(session.id.to_string()).config_options(vec![
            acp::SessionConfigOption::boolean("enabled", "Enabled", true),
        ]),
    );
    session.available_commands = Some(acp::AvailableCommandsUpdate::new(vec![
        acp::AvailableCommand::new("help", "Help"),
    ]));
    assert_eq!(
        SessionMeta::try_from(pb::Session::try_from(&session).expect("wire metadata"))
            .expect("metadata"),
        session
    );
}

#[test]
fn session_preserves_original_initialization_extensions_across_protobuf() {
    let raw = serde_json::json!({
        "protocolVersion": 1,
        "agentCapabilities": {},
        "agentInfo": {"name":"fixture", "version":"1.0.0", "future":"retained"},
        "authMethods": [{
            "id":"future", "name":"Provider authorization", "type":"future-auth",
            "providerField":{"scope":"example"}, "_meta":{"vendor":"retained"}
        }],
        "futureInitialization":{"opaque":9007199254740993_u64}
    });
    let mut session = SessionMeta::new(SessionId::new(), "fixture".into(), Owner::current());
    session.initialization = Some(serde_json::from_value(raw.clone()).expect("typed projection"));
    session.initialization_raw = Some(raw.clone());
    let wire = pb::Session::try_from(&session).expect("wire metadata");
    assert_eq!(
        wire.initialization
            .as_ref()
            .expect("original initialization")
            .decode::<serde_json::Value>()
            .expect("JSON"),
        raw
    );
    assert_eq!(SessionMeta::try_from(wire).expect("metadata"), session);
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

#[cfg(unix)]
#[test]
fn non_utf8_working_directory_is_rejected_without_replacement() {
    use std::os::unix::ffi::OsStringExt;
    let mut session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    session.cwd = Some(std::ffi::OsString::from_vec(b"/tmp/invalid-\xff".to_vec()).into());
    assert!(matches!(
        pb::Session::try_from(&session),
        Err(Error::NonUtf8Cwd)
    ));
    assert!(matches!(
        super::utf8_cwd(session.cwd.as_ref().unwrap()),
        Err(Error::NonUtf8Cwd)
    ));
    session.cwd = Some("/tmp/中文".into());
    assert_eq!(
        pb::Session::try_from(&session).unwrap().cwd.as_deref(),
        Some("/tmp/中文")
    );
}

#[test]
fn turn_receipts_preserve_acknowledged_cursor_and_accept_legacy_default() {
    let turn = TurnId::new();
    for cursor in [Cursor::START, Cursor::from_raw(123)] {
        let receipt = TurnReceipt { turn, cursor };
        let wire = pb::TurnAccepted::from(receipt);
        assert_eq!(wire.turn, turn.to_string());
        assert_eq!(wire.cursor, cursor.raw());
        let decoded = TurnReceipt::try_from(wire).expect("receipt");
        assert_eq!(decoded.turn, turn);
        assert_eq!(decoded.cursor, cursor);
    }
    let legacy = pb::TurnAccepted {
        turn: turn.to_string(),
        ..Default::default()
    };
    assert_eq!(TurnReceipt::try_from(legacy).unwrap().cursor, Cursor::START);
    assert!(matches!(
        TurnReceipt::try_from(pb::TurnAccepted {
            turn: "invalid-turn".into(),
            cursor: 123,
        }),
        Err(Error::Id(_))
    ));
}

#[test]
fn legacy_metadata_defaults_revision_and_active_turn_and_rejects_invalid_turn() {
    let session = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    let mut wire = pb::Session::try_from(&session).unwrap();
    assert_eq!(wire.revision, 0);
    assert_eq!(wire.active_turn, None);
    assert!(!wire.archived);
    assert_eq!(wire.last_reply, 0);
    let decoded = SessionMeta::try_from(wire.clone()).unwrap();
    assert_eq!(decoded.revision, Cursor::START);
    assert_eq!(decoded.active_turn, None);
    assert!(!decoded.archived);
    assert_eq!(decoded.last_reply, Cursor::START);
    wire.active_turn = Some("invalid-turn".into());
    assert!(matches!(SessionMeta::try_from(wire), Err(Error::Id(_))));
}
