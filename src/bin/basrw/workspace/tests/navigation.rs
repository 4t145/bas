use super::*;

#[gpui_kit::test]
fn ctrl_tab_cycles_grouped_threads_and_preserves_drafts_in_both_modes(cx: &mut TestAppContext) {
    for compact in [false, true] {
        let mut fixture = fixture(cx);
        let mut first = session();
        first.cwd = Some("/workspace/a".into());
        let mut second = session();
        second.cwd = Some("/workspace/b".into());
        let mut third = session();
        third.cwd = first.cwd.clone();
        // Sidebar grouping is A: first,third; B: second, not the flat catalog order.
        for meta in [first.clone(), second.clone(), third.clone()] {
            attach(&fixture, meta, cx);
        }
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.select(first.id, window, cx);
                view.resize_window(window, cx);
            });
            window.render_frame(cx);
            window.input("first draft", cx);
        })
        .expect("prepare first thread");
        while fixture.commands.try_recv().is_ok() {}
        for (key, expected, input) in [
            ("ctrl-tab", third.id, Some("third draft")),
            ("ctrl-tab", second.id, Some("second draft")),
            ("ctrl-tab", first.id, None),
            ("ctrl-shift-tab", second.id, None),
        ] {
            cx.update_window(fixture.window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.press(key, cx);
                if let Some(input) = input {
                    window.input(input, cx);
                }
            })
            .expect("thread switch key");
            cx.update(|cx| {
                let view = fixture.view.read(cx);
                assert_eq!(view.active, Some(expected));
                assert_eq!(view.compact, compact);
                if expected == first.id {
                    assert_eq!(view.composer.read(cx).value(), "first draft");
                }
                if expected == second.id {
                    assert_eq!(view.composer.read(cx).value(), "second draft");
                }
            });
        }
        let mut watches = Vec::new();
        while let Ok(command) = fixture.commands.try_recv() {
            match command {
                Command::Watch { session, .. } => watches.push(session),
                Command::Foreground(_) => {}
                _ => panic!("thread switching must not create, prompt, archive or cancel"),
            }
        }
        assert_eq!(watches, [third.id, second.id, first.id, second.id]);
    }
}

#[gpui_kit::test]
fn ctrl_tab_respects_search_and_archive_filters_and_empty_lists(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut first = session();
    first.title = Some("keep first".into());
    let mut second = session();
    second.title = Some("skip second".into());
    let mut archived = session();
    archived.title = Some("keep archived".into());
    archived.archived = true;
    for meta in [first.clone(), second.clone(), archived.clone()] {
        attach(&fixture, meta, cx);
    }
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.search.update(cx, |input, cx| {
                input.set_value("keep", window, cx);
                input.focus(window, cx);
            });
        });
        window.render_frame(cx);
        window.press("ctrl-tab", cx);
    })
    .expect("switch from search");
    cx.update(|cx| assert_eq!(fixture.view.read(cx).active, Some(first.id)));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.archived_list = true;
            cx.notify();
        });
        window.render_frame(cx);
        window.press("ctrl-tab", cx);
    })
    .expect("archived list navigation");
    cx.update(|cx| assert_eq!(fixture.view.read(cx).active, Some(archived.id)));
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.search
                .update(cx, |input, cx| input.set_value("no matches", window, cx));
        });
        window.render_frame(cx);
        window.press("ctrl-tab", cx);
        window.press("ctrl-shift-tab", cx);
    })
    .expect("empty filtered list");
    cx.update(|cx| assert_eq!(fixture.view.read(cx).active, Some(archived.id)));
    assert!(fixture.commands.try_recv().is_err());
}
