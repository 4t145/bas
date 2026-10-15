use super::*;
use crate::workspace::navigation::restore::Store;

#[gpui_kit::test]
fn visible_replies_survive_close_and_reopen_but_new_replies_stay_unread(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("window state");
    let path = directory.path().join("server.json");
    let first = fixture(cx);
    let meta = session();
    attach(&first, meta.clone(), cx);
    cx.update_window(first.window.into(), |_, window, _| window.activate_window())
        .expect("activate reply window");
    cx.run_until_parked();
    let events = completed_events(meta.id, "already read");
    cx.update_window(first.window.into(), |_, window, cx| {
        window.activate_window();
        first.view.update(cx, |view, cx| {
            view.attach_restore(Some(Store::open(path.clone())));
            for event in &events {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: event.clone(),
                    },
                    window,
                    cx,
                );
            }
        });
        window.render_frame(cx);
        assert_eq!(
            first.view.read(cx).sessions.seen_replies.get(&meta.id),
            Some(&2)
        );
        assert!(
            !path.exists(),
            "stream updates debounce instead of writing every frame"
        );
        first
            .view
            .update(cx, |view, cx| view.close_window(window, cx));
    })
    .expect("closing flushes the pending read frontier");
    assert_eq!(
        Store::open(path.clone()).seen_replies().get(&meta.id),
        Some(&2)
    );

    let reopened = fixture(cx);
    cx.update_window(reopened.window.into(), |_, window, cx| {
        reopened.view.update(cx, |view, _| {
            view.attach_restore(Some(Store::open(path.clone())));
            view.window.compact = true;
            view.window.output_open = false;
        });
        window.render_frame(cx);
    })
    .expect("restore without exposing the reply");
    attach(&reopened, meta.clone(), cx);
    cx.update_window(reopened.window.into(), |_, window, cx| {
        reopened.view.update(cx, |view, cx| {
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: meta.clone().into(),
                    events,
                },
                window,
                cx,
            );
            assert!(
                !view.has_unread_reply(),
                "history replay does not forget read state"
            );
            for (index, event) in completed_events(meta.id, "new reply")
                .into_iter()
                .enumerate()
            {
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: Event::new(meta.id, Cursor::from_raw(index as u64 + 4), event.kind),
                    },
                    window,
                    cx,
                );
            }
            assert!(view.has_unread_reply());
            view.mark_thread_read(meta.id, cx);
            assert!(!view.has_unread_reply());
        });
    })
    .expect("new cursors remain unread until explicitly acknowledged");
    assert_eq!(Store::open(path).seen_replies().get(&meta.id), Some(&5));
}

#[gpui_kit::test]
fn read_frontier_is_saved_after_the_debounce_without_closing(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().expect("window state");
    let path = directory.path().join("state.json");
    let fixture = fixture(cx);
    let id = session().id;
    cx.update(|cx| {
        fixture.view.update(cx, |view, cx| {
            view.attach_restore(Some(Store::open(path.clone())));
            view.see_reply(id, 4, cx);
            view.see_reply(id, 8, cx);
            view.see_reply(id, 3, cx);
        })
    });
    assert!(!path.exists());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(Store::open(path).seen_replies().get(&id), Some(&8));
}
