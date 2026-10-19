use super::*;

#[gpui_kit::test]
fn switching_threads_shows_loading_until_the_matching_snapshot_arrives(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let old_epoch = fixture.view.read(cx).selected.epoch;
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries.push(second.clone());
            view.select(second.id, window, cx);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("unsent", window, cx));
            view.composer.expanded = true;
        });
        for width in [1800., 650.] {
            window.resize(size(px(width), px(800.)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            assert!(window.find("conversation-history-loading").visible());
            assert!(window.try_find("empty-conversation-message").is_none());
            let transition = window.find("conversation-history-transition").bounds();
            let composer = window.find("composer-panel").bounds();
            assert_eq!(transition.left(), composer.left());
            assert_eq!(transition.right(), composer.right());
            assert!(transition.size.height > px(0.));
        }
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Snapshot {
                    epoch: old_epoch,
                    session: first.clone().into(),
                    events: Vec::new(),
                },
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(
            window.find("conversation-history-loading").visible(),
            "late old snapshot cannot dismiss loading"
        );
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: second.clone().into(),
                    events: vec![Event::new(
                        second.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(bas::acp::SessionUpdate::UserMessageChunk(
                            bas::acp::ContentChunk::new("Loaded conversation".into()),
                        )),
                    )],
                },
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(window.try_find("conversation-history-transition").is_none());
        assert!(window.try_find("empty-conversation-message").is_none());
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Stream {
                    epoch: view.selected.epoch,
                    state: StreamState::Reconnecting("offline".into()),
                },
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(
            window.try_find("conversation-history-transition").is_none(),
            "reconnect keeps loaded history"
        );
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "unsent"
        );
    })
    .expect("loading transition follows the selected watch epoch");
}

#[gpui_kit::test]
fn unavailable_history_is_not_a_spinner_or_an_empty_conversation(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries.push(meta.clone());
            view.select(meta.id, window, cx);
            view.update(
                Update::Stream {
                    epoch: view.selected.epoch,
                    state: StreamState::Unavailable("unavailable".into()),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("conversation-history-transition").label(),
            Some(crate::i18n::tr!("status-session-unavailable"))
        );
        assert!(window.try_find("conversation-history-loading").is_none());
        assert!(window.try_find("empty-conversation-message").is_none());
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: meta.clone().into(),
                    events: Vec::new(),
                },
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(window.try_find("conversation-history-transition").is_none());
        assert!(
            window.find("empty-conversation-message").visible(),
            "confirmed empty history uses the real empty state"
        );
    })
    .expect("failed and empty history are distinct");
}

#[gpui_kit::test]
fn snapshot_replay_invalidates_rendered_documents_without_erasing_composition(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("unfinished composition", window, cx);
            });
            view.update(
                Update::Snapshot {
                    epoch: view.selected.epoch,
                    session: meta.clone().into(),
                    events: vec![Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                            bas::acp::ContentChunk::new("**rendered history**".into()),
                        )),
                    )],
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        fixture.view.update(cx, |view, cx| {
            assert!(!view.selected.timeline.markdown.borrow().is_empty());
            let generation = view.composer.generation;
            let epoch = view.selected.epoch;
            view.update(
                Update::Snapshot {
                    epoch,
                    session: meta.clone().into(),
                    events: Vec::new(),
                },
                window,
                cx,
            );
            assert!(view.selected.timeline.markdown.borrow().is_empty());
            assert!(view.selected.timeline.images.borrow().is_empty());
            assert!(
                view.selected
                    .timeline
                    .conversation
                    .as_ref()
                    .expect("replayed conversation")
                    .items
                    .is_empty()
            );
            assert_eq!(view.selected.id, Some(meta.id));
            assert_eq!(view.selected.epoch, epoch);
            assert_eq!(view.composer.generation, generation);
            assert_eq!(
                view.composer.input.read(cx).value(),
                "unfinished composition"
            );
        });
    })
    .expect("snapshot replay only replaces history and rendering state");
}
