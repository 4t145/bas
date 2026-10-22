use super::*;
use crate::workspace::navigation::restore::Store;

#[gpui_kit::test]
fn session_switches_and_ui_callbacks_continue_while_persistence_is_gated(cx: &mut TestAppContext) {
    use futures::FutureExt;
    let directory = tempfile::tempdir().expect("window state");
    let path = directory.path().join("server.json");
    let fixture = fixture(cx);
    let first = session();
    let mut last = session();
    last.id = SessionId::new();
    let (release, gate) = futures::channel::oneshot::channel::<()>();
    let blocked = cx
        .background_executor
        .spawn(async move {
            gate.await.expect("release persistence");
        })
        .shared();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.attach_restore(Some(Store::open(path.clone())));
            view.navigation.restore.pending_save = Some(blocked);
            view.sessions.entries = vec![first.clone(), last.clone()];
            view.sessions.seen_replies.insert(first.id, 4);
            view.activate(first.id, false, window, cx);
            view.sessions.seen_replies.insert(first.id, 8);
            view.activate(last.id, false, window, cx);
            assert_eq!(view.selected.id, Some(last.id));
            assert_eq!(
                view.navigation
                    .restore
                    .store
                    .as_ref()
                    .and_then(Store::thread),
                Some(last.id)
            );
        });
    })
    .expect("switch without waiting for disk");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, _, cx| {
        fixture.view.update(cx, |view, _| {
            view.window.sidebar_visible = false;
            assert_eq!(view.selected.id, Some(last.id));
        });
    })
    .expect("independent UI callback runs while persistence is blocked");
    assert!(!path.exists());
    release.send(()).expect("release persistence");
    cx.run_until_parked();
    let saved = Store::open(path);
    assert_eq!(saved.thread(), Some(last.id));
    assert_eq!(saved.seen_replies().get(&first.id), Some(&8));
}

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
                        event: event.clone().into(),
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
    cx.run_until_parked();
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
                history_window(view.selected.epoch, meta.clone().into(), events),
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
                        event: Event::new(meta.id, Cursor::from_raw(index as u64 + 4), event.kind)
                            .into(),
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
    cx.run_until_parked();
    assert_eq!(Store::open(path).seen_replies().get(&meta.id), Some(&5));
}
