use super::*;

#[tokio::test]
async fn child_routing_requires_announcements_and_survives_parent_turn_boundaries() {
    let (sender, mut receiver) = mpsc::channel(8);
    let mut state = State {
        session: Some("root".into()),
        ..State::default()
    };
    let announce = v1::SessionUpdate::SubagentUpdate(
        v1::SubagentUpdate::new("child")
            .state(v1::StateUpdate::Running(v1::RunningStateUpdate::new())),
    );
    assert!(!state.accepts_session(&"child".into()));
    state
        .on_notification(
            v1::SessionNotification::new("root", announce.clone()),
            &sender,
        )
        .await
        .expect("announce");
    assert!(state.accepts_session(&"child".into()));
    assert!(matches!(receiver.recv().await, Some(Update::Acp(update)) if update == announce));
    let chunk = v1::SessionUpdate::AgentMessageChunk(v1::ContentChunk::new("child text".into()));
    for active in [Some(TurnId::new()), None] {
        state.active = active;
        state
            .on_notification(
                v1::SessionNotification::new("child", chunk.clone()),
                &sender,
            )
            .await
            .expect("child message");
        assert!(
            matches!(receiver.recv().await, Some(Update::Subagent(notification)) if notification.session_id == v1::SessionId::new("child"))
        );
    }
    state
        .on_notification(
            v1::SessionNotification::new(
                "child",
                v1::SessionUpdate::SubagentUpdate(v1::SubagentUpdate::new("nested")),
            ),
            &sender,
        )
        .await
        .expect("nested child");
    receiver.recv().await.expect("nested announcement");
    assert!(state.accepts_session(&"nested".into()));
    assert!(!state.accepts_session(&"foreign".into()));
    for (parent, child) in [("root", "nested"), ("child", "root")] {
        assert!(
            state
                .on_notification(
                    v1::SessionNotification::new(
                        parent,
                        v1::SessionUpdate::SubagentUpdate(v1::SubagentUpdate::new(child))
                    ),
                    &sender
                )
                .await
                .is_err()
        );
    }
    assert!(
        state
            .on_notification(v1::SessionNotification::new("foreign", chunk), &sender)
            .await
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
    assert!(Connection::capabilities().subagents.is_some());
}
