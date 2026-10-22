use super::*;

fn capable_session() -> SessionMeta {
    let mut meta = session();
    meta.initialization = Some(
        bas::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
            .agent_capabilities(
                bas::acp::AgentCapabilities::new()
                    .load_session(true)
                    .session_capabilities(
                        bas::acp::SessionCapabilities::new().additional_directories(
                            bas::acp::SessionAdditionalDirectoriesCapabilities::new(),
                        ),
                    ),
            ),
    );
    meta
}

#[gpui_kit::test]
fn directories_reject_busy_unsupported_and_stale_targets_without_optimistic_changes(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let unsupported = session();
    attach(&fixture, unsupported, cx);
    cx.update(|cx| assert!(!fixture.view.read(cx).can_set_directories()));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("session-directories").is_none());
    })
    .expect("unsupported capability hides control");
    let meta = capable_session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("session-directories").visible());
        fixture.view.update(cx, |view, cx| {
            let epoch = view.selected.epoch;
            view.attach_directories(meta.id, epoch.wrapping_sub(1), vec!["/stale".into()], cx);
            assert!(view.can_set_directories());
            view.sessions.pending.configuring.insert(meta.id);
            view.attach_directories(meta.id, epoch, vec!["/busy".into()], cx);
            view.directories_updated(meta.id, Err("Agent rejected roots".into()), cx);
            assert!(
                view.current()
                    .expect("current")
                    .additional_directories
                    .is_empty()
            );
            view.update(
                Update::Event {
                    epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(2),
                        EventKind::TurnStarted {
                            turn: bas::model::TurnId::new(),
                        },
                    )
                    .into(),
                },
                window,
                cx,
            );
            assert!(!view.can_set_directories());
            assert!(view.supports_directories(), "busy is not unsupported");
            view.attach_directories(meta.id, epoch, vec!["/working".into()], cx);
        });
    })
    .expect("workspace operation guards");
    assert!(fixture.commands.try_recv().is_err());
}
