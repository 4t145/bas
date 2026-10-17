use super::{Error, pb};
use crate::{acp, model::*};

mod permissions;
mod submission;

#[test]
fn additional_workspace_metadata_and_events_round_trip() {
    let mut meta = SessionMeta::new(SessionId::new(), "agent".into(), Owner::current());
    let update = Event::new(
        meta.id,
        Cursor::from_raw(1),
        EventKind::WorkspaceChanged {
            directories: vec!["/shared".into()],
            config: acp::NewSessionResponse::new(meta.id.to_string()),
            subagents: Default::default(),
        },
    );
    assert!(meta.apply_event(&update));
    assert_eq!(
        meta.additional_directories,
        [std::path::PathBuf::from("/shared")]
    );
    let wire = pb::Event::from(&update);
    assert_eq!(Event::try_from(wire).expect("event"), update);
    let snapshot = SessionSnapshot::from(meta);
    assert_eq!(
        SessionSnapshot::try_from(pb::Session::try_from(&snapshot).expect("wire"))
            .expect("snapshot"),
        snapshot
    );
}

#[test]
fn agent_reported_timestamp_survives_session_snapshot_transport() {
    let mut meta = SessionMeta::new(SessionId::new(), "test".into(), Owner::current());
    meta.agent_updated_at = Some(Timestamp::from_millis(1234));
    let snapshot = SessionSnapshot::from(meta);
    let wire = pb::Session::try_from(&snapshot).expect("session snapshot");
    assert_eq!(wire.agent_updated_at, Some(1234));
    assert_eq!(SessionSnapshot::try_from(wire).expect("snapshot"), snapshot);
}

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
            result: Err(acp::Error::new(-32000, "authentication required")
                .data(serde_json::json!({"authMethods": [], "large": 9_007_199_254_740_993_u64}))),
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
fn runtime_interruptions_round_trip_without_an_acp_outcome() {
    use prost::Message;
    for reason in [
        InterruptionReason::BackendFailure {
            message: "connection lost".into(),
        },
        InterruptionReason::DispatchFailure {
            message: "cannot dispatch".into(),
        },
        InterruptionReason::SessionStopped,
        InterruptionReason::DaemonRestarted,
        InterruptionReason::CancelledBeforeDispatch,
    ] {
        let original = event(EventKind::TurnInterrupted {
            turn: TurnId::new(),
            reason,
        });
        let wire = pb::Event::from(&original);
        assert!(matches!(
            wire.kind,
            Some(pb::event::Kind::TurnInterrupted(_))
        ));
        let decoded = pb::Event::decode(wire.encode_to_vec().as_slice()).expect("protobuf event");
        assert_eq!(Event::try_from(decoded).expect("interruption"), original);
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
    let wire = pb::Event {
        session: SessionId::new().to_string(),
        kind: Some(pb::event::Kind::TurnFinished(pb::TurnFinished {
            turn: TurnId::new().to_string(),
            outcome: Some(pb::turn_finished::Outcome::Error(pb::AcpPayload {
                json: br#""unclassified failure""#.to_vec(),
            })),
        })),
        ..Default::default()
    };
    assert!(matches!(Event::try_from(wire), Err(Error::Json(_))));
    for reason in [pb::InterruptionReason::Unspecified as i32, 999] {
        let wire = pb::Event {
            session: SessionId::new().to_string(),
            kind: Some(pb::event::Kind::TurnInterrupted(pb::TurnInterrupted {
                turn: TurnId::new().to_string(),
                reason,
                message: String::new(),
            })),
            ..Default::default()
        };
        assert!(matches!(
            Event::try_from(wire),
            Err(Error::Unknown {
                field: "interruption reason",
                ..
            })
        ));
    }
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
    let snapshot = SessionSnapshot::from(session);
    assert_eq!(
        SessionSnapshot::try_from(pb::Session::try_from(&snapshot).expect("wire snapshot"))
            .expect("snapshot"),
        snapshot
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
    let snapshot = SessionSnapshot::from(session);
    let wire = pb::Session::try_from(&snapshot).expect("wire snapshot");
    assert_eq!(
        wire.initialization
            .as_ref()
            .expect("original initialization")
            .decode::<serde_json::Value>()
            .expect("JSON"),
        raw
    );
    assert_eq!(SessionSnapshot::try_from(wire).expect("snapshot"), snapshot);
}

#[test]
fn missing_owner_and_unknown_lifecycle_status_are_rejected() {
    let session = pb::Session {
        id: SessionId::new().to_string(),
        status: pb::SessionStatus::Idle as i32,
        ..Default::default()
    };
    assert!(matches!(
        SessionSnapshot::try_from(session.clone()),
        Err(Error::MissingOwner)
    ));
    let session = pb::Session {
        owner: Some(pb::Owner::default()),
        status: 999,
        ..session
    };
    assert!(matches!(
        SessionSnapshot::try_from(session),
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
        pb::Session::try_from(&SessionSnapshot::from(session.clone())),
        Err(Error::NonUtf8Cwd)
    ));
    assert!(matches!(
        super::utf8_cwd(session.cwd.as_ref().unwrap()),
        Err(Error::NonUtf8Cwd)
    ));
    session.cwd = Some("/tmp/中文".into());
    assert_eq!(
        pb::Session::try_from(&SessionSnapshot::from(session))
            .expect("UTF-8 directory")
            .cwd
            .as_deref(),
        Some("/tmp/中文")
    );
}

#[test]
fn snapshot_preserves_initial_metadata_and_rejects_invalid_turn() {
    let session = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "test".into(),
        Owner::current(),
    ));
    let mut wire = pb::Session::try_from(&session).unwrap();
    assert_eq!(wire.revision, 0);
    assert_eq!(wire.active_turn, None);
    assert!(!wire.archived);
    assert_eq!(wire.last_reply, 0);
    let decoded = SessionSnapshot::try_from(wire.clone())
        .expect("snapshot")
        .meta;
    assert_eq!(decoded.revision, Cursor::START);
    assert_eq!(decoded.active_turn, None);
    assert!(!decoded.archived);
    assert_eq!(decoded.last_reply, Cursor::START);
    wire.active_turn = Some("invalid-turn".into());
    assert!(matches!(SessionSnapshot::try_from(wire), Err(Error::Id(_))));
}

#[test]
fn runtime_failure_round_trips_independently_of_committed_status_and_revision() {
    use prost::Message;
    let mut meta = SessionMeta::new(SessionId::new(), "fixture".into(), Owner::current());
    meta.status = SessionStatus::Working;
    meta.active_turn = Some(TurnId::new());
    meta.revision = Cursor::from_raw(42);
    let mut snapshot = SessionSnapshot::from(meta.clone());
    snapshot.runtime.failure = Some(RuntimeFailure::Persistence {
        message: "fixture write failure".into(),
    });
    let wire = pb::Session::try_from(&snapshot).expect("snapshot");
    assert_eq!(wire.status(), pb::SessionStatus::Working);
    assert_eq!(wire.revision, 42);
    let bytes = wire.encode_to_vec();
    let decoded =
        SessionSnapshot::try_from(pb::Session::decode(bytes.as_slice()).expect("protobuf"))
            .expect("snapshot");
    assert_eq!(decoded, snapshot);
    assert_eq!(decoded.meta, meta);
    assert_eq!(decoded.status(), SessionStatus::Failed);
    assert_eq!(decoded.active_turn(), None);
}

#[test]
fn missing_or_malformed_runtime_state_is_never_treated_as_healthy() {
    let snapshot = SessionSnapshot::from(SessionMeta::new(
        SessionId::new(),
        "fixture".into(),
        Owner::current(),
    ));
    let mut wire = pb::Session::try_from(&snapshot).expect("snapshot");
    wire.runtime = None;
    assert!(matches!(
        SessionSnapshot::try_from(wire),
        Err(Error::MissingRuntime)
    ));
    let mut wire = pb::Session::try_from(&snapshot).expect("snapshot");
    wire.runtime.as_mut().expect("runtime").id = "invalid".into();
    assert!(matches!(
        SessionSnapshot::try_from(wire),
        Err(Error::Unknown {
            field: "session runtime id",
            ..
        })
    ));
    for kind in [pb::runtime_failure::Kind::Unspecified as i32, 999] {
        let mut wire = pb::Session::try_from(&snapshot).expect("snapshot");
        wire.runtime.as_mut().expect("runtime").failure = Some(pb::RuntimeFailure {
            kind,
            message: "fixture".into(),
        });
        assert!(matches!(
            SessionSnapshot::try_from(wire),
            Err(Error::Unknown {
                field: "runtime failure kind",
                ..
            })
        ));
    }
}
