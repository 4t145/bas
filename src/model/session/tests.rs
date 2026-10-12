use super::*;

fn metadata() -> SessionMeta {
    SessionMeta::new(SessionId::new(), "test".into(), Owner::current())
}

#[test]
fn agent_timestamp_survives_event_clock_updates_and_explicit_clear() {
    let mut meta = metadata();
    let update: crate::acp::SessionUpdate = serde_json::from_value(serde_json::json!({
        "sessionUpdate": "session_info_update",
        "title": "Agent title",
        "updatedAt": "2026-10-10T08:00:00+08:00"
    }))
    .expect("session info update");
    assert!(meta.apply_event(&Event::new(
        meta.id,
        Cursor::from_raw(1),
        EventKind::Acp(update)
    )));
    let reported = meta.agent_updated_at.expect("valid agent timestamp");
    assert_eq!(reported.millis(), 1_791_590_400_000);
    assert!(meta.apply_event(&Event::new(
        meta.id,
        Cursor::from_raw(2),
        EventKind::ArchiveChanged { archived: true }
    )));
    assert_eq!(meta.agent_updated_at, Some(reported));
    let invalid: crate::acp::SessionUpdate = serde_json::from_value(serde_json::json!({
        "sessionUpdate": "session_info_update", "updatedAt": "invalid"
    }))
    .expect("invalid timestamp remains an SDK string");
    meta.apply_acp(&invalid);
    assert_eq!(meta.agent_updated_at, Some(reported));
    let clear: crate::acp::SessionUpdate = serde_json::from_value(serde_json::json!({
        "sessionUpdate": "session_info_update", "updatedAt": null
    }))
    .expect("explicit clear");
    meta.apply_acp(&clear);
    assert_eq!(meta.agent_updated_at, None);
}

#[test]
fn stale_or_foreign_events_cannot_overwrite_a_newer_snapshot() {
    let mut meta = metadata();
    let active = TurnId::new();
    let started = Event::new(
        meta.id,
        Cursor::from_raw(10),
        EventKind::TurnStarted { turn: active },
    );
    assert!(meta.apply_event(&started));
    let snapshot = meta.clone();

    for event in [
        started,
        Event::new(
            meta.id,
            Cursor::from_raw(9),
            EventKind::StatusChanged {
                status: SessionStatus::Idle,
            },
        ),
        Event::new(
            SessionId::new(),
            Cursor::from_raw(11),
            EventKind::TitleChanged {
                title: "another session".into(),
            },
        ),
    ] {
        assert!(!meta.apply_event(&event));
        assert_eq!(meta, snapshot);
    }
    assert_eq!(meta.active_turn, Some(active));
}

#[test]
fn metadata_order_uses_event_revision_not_wall_clock_time() {
    let mut meta = metadata();
    let mut first = Event::new(
        meta.id,
        Cursor::from_raw(1),
        EventKind::TitleChanged {
            title: "before clock adjustment".into(),
        },
    );
    first.at = Timestamp::from_millis(100);
    assert!(meta.apply_event(&first));

    let mut second = Event::new(
        meta.id,
        Cursor::from_raw(2),
        EventKind::TitleChanged {
            title: "after clock adjustment".into(),
        },
    );
    second.at = Timestamp::from_millis(90);
    assert!(meta.apply_event(&second));
    assert_eq!(meta.title.as_deref(), Some("after clock adjustment"));
    assert_eq!(meta.revision, Cursor::from_raw(2));
    assert!(!meta.apply_event(&first));
}

#[test]
fn completion_and_terminal_states_clear_the_active_turn() {
    let mut meta = metadata();
    let turn = TurnId::new();
    assert!(meta.apply_event(&Event::new(
        meta.id,
        Cursor::from_raw(1),
        EventKind::TurnStarted { turn },
    )));
    assert!(meta.apply_event(&Event::new(
        meta.id,
        Cursor::from_raw(2),
        EventKind::TurnFinished {
            turn,
            result: Ok(crate::acp::PromptResponse::new(
                crate::acp::StopReason::EndTurn
            )),
        },
    )));
    assert_eq!(meta.status, SessionStatus::Idle);
    assert_eq!(meta.active_turn, None);

    assert!(meta.apply_event(&Event::new(
        meta.id,
        Cursor::from_raw(3),
        EventKind::TurnStarted {
            turn: TurnId::new()
        },
    )));
    assert!(meta.apply_event(&Event::new(
        meta.id,
        Cursor::from_raw(4),
        EventKind::Failed {
            message: "backend disconnected".into(),
        },
    )));
    assert_eq!(meta.status, SessionStatus::Failed);
    assert_eq!(meta.active_turn, None);
}

#[test]
fn older_serialized_metadata_defaults_to_an_unversioned_snapshot() {
    let mut value = serde_json::to_value(metadata()).expect("metadata should serialize");
    let object = value.as_object_mut().expect("metadata is an object");
    object.remove("revision");
    object.remove("archived");
    object.remove("last_reply");
    let decoded: SessionMeta =
        serde_json::from_value(value).expect("older metadata should remain readable");
    assert_eq!(decoded.revision, Cursor::START);
    assert_eq!(decoded.active_turn, None);
    assert!(!decoded.archived);
    assert_eq!(decoded.last_reply, Cursor::START);
}

#[test]
fn unread_progress_tracks_only_new_agent_messages() {
    let mut meta = metadata();
    let updates = [
        EventKind::TitleChanged {
            title: "renamed".into(),
        },
        EventKind::Acp(crate::acp::SessionUpdate::AgentThoughtChunk(
            crate::acp::ContentChunk::new("thinking".into()),
        )),
        EventKind::Acp(crate::acp::SessionUpdate::UserMessageChunk(
            crate::acp::ContentChunk::new("question".into()),
        )),
        EventKind::Acp(crate::acp::SessionUpdate::AgentMessageChunk(
            crate::acp::ContentChunk::new("reply".into()),
        )),
        EventKind::ArchiveChanged { archived: true },
    ];
    for (index, kind) in updates.into_iter().enumerate() {
        let cursor = Cursor::from_raw(index as u64 + 1);
        assert!(meta.apply_event(&Event::new(meta.id, cursor, kind)));
        assert_eq!(meta.last_reply.raw(), if index < 3 { 0 } else { 4 });
    }
    assert!(!meta.apply_event(&Event::new(
        meta.id,
        Cursor::from_raw(3),
        EventKind::Acp(crate::acp::SessionUpdate::AgentMessageChunk(
            crate::acp::ContentChunk::new("late".into()),
        )),
    )));
    assert_eq!(meta.last_reply.raw(), 4);
}

#[test]
fn archive_is_independent_of_lifecycle_and_rejects_stale_restores() {
    for status in [
        SessionStatus::Starting,
        SessionStatus::Idle,
        SessionStatus::Working,
        SessionStatus::Stopped,
        SessionStatus::Failed,
    ] {
        let mut meta = metadata();
        meta.status = status;
        meta.active_turn = (status == SessionStatus::Working).then(TurnId::new);
        let active_turn = meta.active_turn;
        let archived = Event::new(
            meta.id,
            Cursor::from_raw(2),
            EventKind::ArchiveChanged { archived: true },
        );
        assert!(meta.apply_event(&archived));
        assert!(meta.archived);
        assert_eq!(meta.status, status);
        assert_eq!(meta.active_turn, active_turn);
        assert_eq!(meta.revision, archived.cursor);
        assert_eq!(meta.updated_at, archived.at);
        for stale in [
            Event::new(
                meta.id,
                Cursor::from_raw(1),
                EventKind::ArchiveChanged { archived: false },
            ),
            Event::new(
                SessionId::new(),
                Cursor::from_raw(3),
                EventKind::ArchiveChanged { archived: false },
            ),
        ] {
            assert!(!meta.apply_event(&stale));
            assert!(meta.archived);
        }
        assert!(meta.apply_event(&Event::new(
            meta.id,
            Cursor::from_raw(3),
            EventKind::ArchiveChanged { archived: false },
        )));
        assert!(!meta.archived);
        assert_eq!(meta.status, status);
        assert_eq!(meta.active_turn, active_turn);
    }
}

#[test]
fn finishing_an_archived_turn_does_not_restore_the_session() {
    let mut meta = metadata();
    let turn = TurnId::new();
    for (cursor, kind) in [
        (1, EventKind::TurnStarted { turn }),
        (2, EventKind::ArchiveChanged { archived: true }),
        (
            3,
            EventKind::TurnFinished {
                turn,
                result: Ok(crate::acp::PromptResponse::new(
                    crate::acp::StopReason::EndTurn,
                )),
            },
        ),
    ] {
        assert!(meta.apply_event(&Event::new(meta.id, Cursor::from_raw(cursor), kind)));
    }
    assert!(meta.archived);
    assert_eq!(meta.status, SessionStatus::Idle);
    assert_eq!(meta.active_turn, None);
}

#[test]
fn runtime_interruption_clears_activity_without_inventing_an_acp_result() {
    let mut meta = metadata();
    let turn = TurnId::new();
    for (cursor, kind) in [
        (1, EventKind::TurnStarted { turn }),
        (2, EventKind::ArchiveChanged { archived: true }),
        (
            3,
            EventKind::TurnInterrupted {
                turn,
                reason: crate::model::InterruptionReason::DaemonRestarted,
            },
        ),
    ] {
        assert!(meta.apply_event(&Event::new(meta.id, Cursor::from_raw(cursor), kind)));
    }
    assert!(meta.archived);
    assert_eq!(meta.status, SessionStatus::Idle);
    assert_eq!(meta.active_turn, None);
    assert_eq!(meta.revision, Cursor::from_raw(3));
    let failure = Event::new(
        meta.id,
        Cursor::from_raw(4),
        EventKind::Failed {
            message: "backend unavailable".into(),
        },
    );
    assert!(meta.apply_event(&failure));
    assert_eq!(meta.status, SessionStatus::Failed);
    assert_eq!(meta.active_turn, None);
}
