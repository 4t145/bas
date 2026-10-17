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
fn adding_directories_targets_the_existing_session_and_retains_the_draft(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = capable_session();
    attach(&fixture, meta.clone(), cx);
    let extra = tempfile::tempdir().expect("extra root");
    let popup = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer
                    .input
                    .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            });
            while fixture.commands.try_recv().is_ok() {}
            window.render_frame(cx);
            window.click("session-directories", cx);
            fixture
                .view
                .read(cx)
                .window
                .menu
                .as_ref()
                .and_then(|menu| menu.window)
                .unwrap_or(fixture.window.into())
        })
        .expect("open session roots");
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .expect("attach folder menu");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.simulate_next_frame(cx)
    })
    .expect("activate parent");
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && options.multiple && !options.files);
        Some(vec![extra.path().into()])
    });
    cx.run_until_parked();
    let requests: Vec<_> = std::iter::from_fn(|| fixture.commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::SetDirectories {
                session,
                directories,
            } => Some((session, directories)),
            Command::Foreground(_) => None,
            _ => panic!("attaching a root must not create or detach a session"),
        })
        .collect();
    assert_eq!(requests, vec![(meta.id, vec![extra.path().to_owned()])]);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.selected.id, Some(meta.id));
            assert_eq!(view.current().expect("current").cwd, meta.cwd);
            assert!(
                view.current()
                    .expect("current")
                    .additional_directories
                    .is_empty()
            );
            assert!(!view.can_set_directories());
            let mut updated = meta.clone();
            updated.additional_directories = vec![extra.path().into()];
            updated.revision = Cursor::from_raw(2);
            view.update(
                Update::DirectoriesUpdated {
                    session: meta.id,
                    result: Ok(updated.into()),
                },
                window,
                cx,
            );
            assert_eq!(
                view.current().expect("current").additional_directories,
                [extra.path()]
            );
            assert_eq!(view.composer.input.read(cx).value(), "keep draft");
        });
    })
    .expect("confirmed workspace update");
}

#[gpui_kit::test]
fn directories_reject_busy_unsupported_and_stale_targets_without_optimistic_changes(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let unsupported = session();
    attach(&fixture, unsupported, cx);
    cx.update(|cx| assert!(!fixture.view.read(cx).can_set_directories()));
    let meta = capable_session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
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
                    ),
                },
                window,
                cx,
            );
            assert!(!view.can_set_directories());
            view.attach_directories(meta.id, epoch, vec!["/working".into()], cx);
        });
    })
    .expect("workspace operation guards");
    assert!(fixture.commands.try_recv().is_err());
}
