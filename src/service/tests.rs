use super::{Error, pb};
use crate::{acp, model::*};

mod permissions;
mod submission;

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
