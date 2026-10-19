use super::*;

fn metadata() -> SessionMeta {
    SessionMeta::new(SessionId::new(), "test".into(), Owner::current())
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
