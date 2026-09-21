use super::*;

#[gpui_kit::test]
fn title_edits_in_place_and_escape_restores_focus(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let header = window.find("thread-header").bounds();
        let slot = window.find("thread-title-slot").bounds();
        window.click("rename-session", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).editing_title);
        assert_eq!(window.find("thread-header").bounds(), header);
        assert_eq!(window.find("thread-title-slot").bounds(), slot);
        window.press("escape", cx);
        assert!(!fixture.view.read(cx).editing_title);
        assert!(
            fixture
                .view
                .read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    })
    .expect("window");
}

#[gpui_kit::test]
fn enter_guards_empty_and_pending_and_preserves_failed_input(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.begin_rename(window, cx);
            view.rename
                .update(cx, |input, cx| input.set_value("  ", window, cx));
        });
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .expect("window");
    assert!(fixture.commands.try_recv().is_err());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert!(fixture.commands.try_recv().is_err());
        fixture.view.update(cx, |view, cx| {
            view.rename
                .update(cx, |input, cx| input.set_value("  changed  ", window, cx));
        });
        window.press("enter", cx);
        window.press("enter", cx);
    })
    .expect("window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Rename {session, title}) if session == meta.id && title == "changed"));
        assert!(fixture.commands.try_recv().is_err());
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Renamed {session: meta.id, result: Err("offline".into())}, window, cx);
            assert!(view.editing_title);
            assert!(!view.renaming);
            assert_eq!(view.rename.read(cx).value().as_str(), "  changed  ");
            assert!(view.error.as_ref().is_some_and(|error| error.contains("offline")));
            assert_eq!(view.current().expect("session").title, meta.title);
        });
        window.press("enter", cx);
    }).expect("window");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert!(matches!(
            fixture.commands.try_recv(),
            Ok(Command::Rename { .. })
        ));
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Renamed {
                    session: meta.id,
                    result: Ok(()),
                },
                window,
                cx,
            );
            assert!(!view.editing_title);
            assert_eq!(view.current().expect("session").title, meta.title);
        });
    })
    .expect("window");
}

#[gpui_kit::test]
fn switching_thread_cancels_old_input_and_ignores_background_failure(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    attach(&fixture, first.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.begin_rename(window, cx);
            view.rename
                .update(cx, |input, cx| input.set_value("old draft", window, cx));
            view.save_title(cx);
        });
    })
    .expect("window");
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(!view.editing_title);
            view.save_title(cx);
            view.begin_rename(window, cx);
            assert!(
                !view.editing_title,
                "pending rename must settle before another edit"
            );
            view.update(
                Update::Renamed {
                    session: first.id,
                    result: Err("offline".into()),
                },
                window,
                cx,
            );
            assert!(view.error.is_none());
            view.begin_rename(window, cx);
            assert_ne!(view.rename.read(cx).value().as_str(), "old draft");
        });
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("window");
}
