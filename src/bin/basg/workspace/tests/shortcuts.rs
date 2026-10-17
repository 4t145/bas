use super::*;

#[gpui_kit::test]
fn ctrl_s_stops_without_confirmation_and_archived_threads_only_restore(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| input.set_value("unsent", window, cx));
            view.focus_input(window, cx);
        });
        window.press("ctrl-s", cx);
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::CloseSession(id)) if id == meta.id));
        assert_eq!(cx.windows().len(), 1, "stop does not ask for confirmation");
        meta.status = SessionStatus::Stopped;
        meta.revision = Cursor::from_raw(1);
        fixture.view.update(cx, |view, cx| view.update(Update::ClosedSession { session: meta.id, result: Ok(meta.clone().into()) }, window, cx));
        window.press("ctrl-s", cx);
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::RestartSession(id)) if id == meta.id));
        meta.status = SessionStatus::Idle;
        meta.revision = Cursor::from_raw(2);
        fixture.view.update(cx, |view, cx| view.update(Update::RestartedSession { session: meta.id, result: Ok(meta.clone().into()) }, window, cx));
        while fixture.commands.try_recv().is_ok() {}
        meta.archived = true;
        meta.status = SessionStatus::Stopped;
        meta.revision = Cursor::from_raw(3);
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries[0] = meta.clone();
            view.selected.timeline.conversation.as_mut().expect("conversation").session = meta.clone();
            cx.notify();
        });
        window.press("ctrl-s", cx);
        assert!(fixture.commands.try_recv().is_err(), "archived session cannot be directly started");
        window.press("ctrl-w", cx);
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Archive { session, archived: false }) if session == meta.id));
        assert_eq!(fixture.view.read(cx).composer.input.read(cx).value(), "unsent");
        window.render_frame(cx);
        assert!(window.find("server-status").label().expect("connection").contains("127.0.0.1:7337"));
        assert_eq!(window.find("server-status").bounds().center().y, window.find("composer-toolbar").bounds().center().y);
    }).expect("session lifecycle keyboard controls");
}

#[gpui_kit::test]
fn thread_shortcuts_work_from_composer_without_editing_or_sending_the_draft(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| input.set_value("keep these words", window, cx));
            view.toggle_mode(window, cx);
            view.focus_input(window, cx);
        });
        window.render_frame(cx);
        window.press("f2", cx);
        assert!(!fixture.view.read(cx).window.compact, "rename exposes the title editor");
        assert!(matches!(fixture.view.read(cx).renaming.edit.as_ref().map(|edit| &edit.target), Some(crate::workspace::rename::Target::Thread(id)) if *id == meta.id));
        window.press("escape", cx);
        window.press("ctrl-alt-p", cx);
        assert!(fixture.view.read(cx).navigation.pinned.contains(&meta.id));
        window.press("ctrl-alt-p", cx);
        assert!(!fixture.view.read(cx).navigation.pinned.contains(&meta.id));
        window.press("ctrl-w", cx);
        window.press("ctrl-w", cx);
        assert_eq!(fixture.view.read(cx).composer.input.read(cx).value(), "keep these words");
    }).expect("thread actions override editor bindings");
    let mut archives = 0;
    while let Ok(command) = fixture.commands.try_recv() {
        match command {
            Command::Archive {
                session,
                archived: true,
            } => {
                assert_eq!(session, meta.id);
                archives += 1;
            }
            Command::Foreground(_) => {}
            _ => panic!("shortcuts must not send, delete or alter the draft"),
        }
    }
    assert_eq!(archives, 1, "pending archive is deduplicated");
    assert!(
        cx.update_window(fixture.window.into(), |_, _, _| ())
            .is_ok()
    );
}

#[gpui_kit::test]
fn delete_shortcut_requires_confirmation_and_keeps_the_original_target(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        window.render_frame(cx);
        window.press("ctrl-alt-w", cx);
    })
    .expect("request deletion");
    while let Ok(command) = fixture.commands.try_recv() {
        assert!(
            matches!(command, Command::Foreground(_)),
            "no delete before confirmation"
        );
    }
    let confirmation = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| handle.window_id() != fixture.window.window_id())
            .expect("confirmation window")
    });
    cx.update_window(confirmation, |_, window, cx| {
        window.render_frame(cx);
        window.click("message-confirm", cx);
    })
    .expect("confirm deletion");
    assert!(matches!(fixture.commands.try_recv(), Ok(Command::Delete(id)) if id == meta.id));
}

#[gpui_kit::test]
fn workspace_shortcut_opens_folder_picker_and_cancel_keeps_the_thread(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.activate_window();
        fixture
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        window.render_frame(cx);
        window.press("ctrl-alt-n", cx);
    })
    .expect("request workspace");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
    })
    .expect("activation frame");
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert_eq!(
        fixture.view.read_with(cx, |view, _| view.selected.id),
        Some(meta.id)
    );
}

#[gpui_kit::test]
fn settings_close_shortcut_does_not_archive_the_workspace(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let settings = cx.update(|cx| crate::settings::window::open(cx).expect("settings"));
    cx.update_window(settings.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-search", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
    })
    .expect("close only settings");
    assert!(cx.update_window(settings.into(), |_, _, _| ()).is_err());
    while let Ok(command) = fixture.commands.try_recv() {
        assert!(!matches!(
            command,
            Command::Archive { .. } | Command::Delete(_)
        ));
    }
}
