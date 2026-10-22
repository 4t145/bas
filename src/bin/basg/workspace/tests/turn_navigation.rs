use super::*;
use bas::service::{HistoryLocation, HistoryOverview, HistoryOverviewEntry, HistoryWindow};
use gpui_kit::InputEvent as _;
use gpui_kit::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ScrollDelta, ScrollWheelEvent, point,
};

fn body(meta: &SessionMeta, first: u64, last: u64, frontier: u64) -> HistoryWindow {
    let mut snapshot = meta.clone();
    snapshot.revision = Cursor::from_raw(frontier);
    HistoryWindow {
        session: snapshot.into(),
        events: (first..=last)
            .map(|cursor| {
                let chunk = bas::acp::ContentChunk::new(format!("Message {cursor}").into());
                Event::new(
                    meta.id,
                    Cursor::from_raw(cursor),
                    EventKind::Acp(if cursor % 2 == 1 {
                        bas::acp::SessionUpdate::UserMessageChunk(chunk)
                    } else {
                        bas::acp::SessionUpdate::AgentMessageChunk(chunk)
                    }),
                )
            })
            .collect(),
        context: Vec::new(),
        before: (first > 1).then_some(first),
        cursor: frontier,
    }
}

fn overview(meta: &SessionMeta, frontier: u64) -> HistoryOverview {
    HistoryOverview {
        session: meta.id,
        cursor: frontier,
        entries: (1..=frontier)
            .step_by(2)
            .map(|cursor| HistoryOverviewEntry {
                cursor,
                timestamp_millis: bas::model::Timestamp::now()
                    .millis()
                    .saturating_sub(5 * 60 * 1000) as i64,
                preview: format!("Prompt {cursor}"),
            })
            .collect(),
    }
}

fn show_latest(fixture: &Fixture, meta: &SessionMeta, frontier: u64, cx: &mut TestAppContext) {
    attach(fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::HistoryWindow {
                    epoch: view.selected.epoch,
                    window: body(meta, frontier - 1, frontier, frontier).into(),
                    cached: false,
                },
                window,
                cx,
            );
            let request = view
                .selected
                .timeline
                .navigation
                .overview_request
                .expect("requested entire catalog");
            view.update(
                Update::HistoryOverview {
                    session: meta.id,
                    epoch: view.selected.epoch,
                    request,
                    result: Ok(overview(meta, frontier).into()),
                },
                window,
                cx,
            );
            cx.notify();
        });
        window.render_frame(cx);
    })
    .expect("latest body and whole overview");
}

fn location(
    view: &mut Workspace,
    meta: &SessionMeta,
    first: u64,
    last: u64,
    frontier: u64,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let request = view
        .selected
        .timeline
        .navigation
        .location
        .expect("location request");
    view.update(
        Update::HistoryLocation {
            session: meta.id,
            epoch: view.selected.epoch,
            request: request.request,
            result: Ok(HistoryLocation {
                window: body(meta, first, last, frontier),
                after: (last < frontier).then_some(last + 1),
            }
            .into()),
        },
        window,
        cx,
    );
}

#[gpui_kit::test]
fn whole_overview_appears_without_backfilling_and_ctrl_click_only_bookmarks(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    let saved = tempfile::tempdir().expect("navigation state");
    cx.update(|cx| {
        fixture.view.update(cx, |view, _| {
            view.attach_restore(Some(workspace_navigation::restore::Store::open(
                saved.path().join("window.json"),
            )));
        })
    });
    show_latest(&fixture, &meta, 100, cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert!(
            window.find("turn-marker-1").visible(),
            "oldest is present without old body"
        );
        assert!(window.find("turn-marker-99").visible());
        assert!(
            window.try_find("turn-navigation-count").is_none(),
            "no unsolicited count label"
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .history
                .as_ref()
                .expect("body")
                .events
                .len(),
            2
        );
        let before = window.find("turn-marker-line-1").bounds().size;
        let position = window.find("turn-marker-1").bounds().center();
        let modifiers = Modifiers {
            control: true,
            ..Default::default()
        };
        window.dispatch_event(
            MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                modifiers,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseUpEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                modifiers,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(fixture.view.read(cx).navigation.marker_bookmarks[&meta.id].contains(&1));
        assert!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .navigation
                .location
                .is_none(),
            "Ctrl-click does not jump"
        );
        window.hover("turn-marker-99", cx);
        window.render_frame(cx);
        assert_eq!(window.find("turn-marker-line-1").bounds().size, before);
        let hint = window
            .find("turn-marker-1")
            .label()
            .expect("hover/accessibility hint")
            .to_owned();
        assert!(hint.contains("Ctrl") && hint.contains("Alt"));
        assert!(hint.contains(&crate::i18n::tr!("time-minutes", count = 5)));
        assert!(!hint.contains("UTC"));
        window.hover("turn-marker-1", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("turn-marker-line-1").bounds().size,
            size(px(20.), px(3.)),
            "hover still expands the tick"
        );
    })
    .expect("bookmark interaction");
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("turn-marker-bookmark-label").visible());
        assert!(window.find("turn-marker-bookmark-icon").visible());
        assert!(
            window
                .find("turn-marker-bookmark-label")
                .bounds()
                .size
                .width
                < window.find("turn-marker-metadata").bounds().size.width,
            "bookmark is a compact icon badge, not a full-width strip"
        );
        let relative_time = crate::i18n::tr!("time-minutes", count = 5);
        assert_eq!(
            window.find("turn-marker-time").label(),
            Some(relative_time.as_str())
        );
        let preview = window.find("turn-marker-preview").bounds();
        let metadata = window.find("turn-marker-metadata").bounds();
        let shortcuts = window.find("turn-marker-shortcuts").bounds();
        assert!(
            preview.bottom() < metadata.top() && metadata.bottom() < shortcuts.top(),
            "preview, metadata and shortcuts have separate sections"
        );
        let previous = super::super::shortcuts::hint(
            crate::i18n::tr!("turn-marker-previous"),
            &super::super::shortcuts::PreviousConversationTurn,
            window,
        );
        let next = super::super::shortcuts::hint(
            crate::i18n::tr!("turn-marker-next"),
            &super::super::shortcuts::NextConversationTurn,
            window,
        );
        assert_eq!(
            window.find("turn-marker-previous-hint").label(),
            Some(previous.as_str())
        );
        assert_eq!(
            window.find("turn-marker-next-hint").label(),
            Some(next.as_str())
        );
        assert!(
            previous.contains("Ctrl") && next.contains("Ctrl"),
            "hints include actual keyboard bindings"
        );
        window.click("turn-marker-1", cx);
        window.hover("turn-marker-99", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("turn-marker-line-1").bounds().size.width,
            px(14.)
        );
    })
    .expect("bookmarked hover and direct jump");
    let commands: Vec<_> = std::iter::from_fn(|| fixture.commands.try_recv().ok()).collect();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::HistoryLocation { cursor: 1, .. }))
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::OlderHistory { .. }))
    );
    let restored = workspace_navigation::restore::Store::open(saved.path().join("window.json"));
    assert!(restored.marker_bookmarks()[&meta.id].contains(&1));
}

#[gpui_kit::test]
fn current_marker_is_bold_without_lengthening_or_confusing_bookmarks(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    show_latest(&fixture, &meta, 6, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigation
                .marker_bookmarks
                .insert(meta.id, HashSet::from([1]));
            view.navigate_to_marker(5, cx);
        });
        window.render_frame(cx);
        window.hover("turn-marker-end", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("turn-marker-line-3").bounds().size,
            size(px(14.), px(2.))
        );
        assert_eq!(
            window.find("turn-marker-line-1").bounds().size,
            size(px(14.), px(2.)),
            "an inactive bookmark stays thin"
        );
        assert_eq!(
            window.find("turn-marker-line-5").bounds().size,
            size(px(14.), px(4.)),
            "the current marker is emphasized without becoming longer"
        );
        window.hover("turn-marker-5", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("turn-marker-line-5").bounds().size,
            size(px(20.), px(4.)),
            "hover extends the current marker without reducing its emphasis"
        );
        window.click("turn-marker-end", cx);
        window.hover("turn-marker-3", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("turn-marker-line-end").bounds().size,
            size(px(14.), px(4.))
        );
        assert_eq!(
            window.find("turn-marker-line-5").bounds().size,
            size(px(14.), px(2.)),
            "the previous selection loses its current-state emphasis"
        );
    })
    .expect("distinct current, bookmark and hover states");
}

#[gpui_kit::test]
fn control_page_keys_use_the_same_marker_navigation(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    show_latest(&fixture, &meta, 20, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.press("ctrl-pageup", cx);
        assert_eq!(
            fixture.view.read(cx).selected.timeline.navigation.selected,
            Some(19)
        );
        window.press("ctrl-pageup", cx);
        assert_eq!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .navigation
                .location
                .expect("previous unloaded marker")
                .cursor,
            17
        );
        window.press("ctrl-pagedown", cx);
        assert_eq!(
            fixture.view.read(cx).selected.timeline.navigation.selected,
            Some(19)
        );
        assert!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .navigation
                .location
                .is_none()
        );
    })
    .expect("keyboard marker navigation");
}

#[gpui_kit::test]
fn direct_jump_separates_live_events_and_forward_pages_join_without_gaps(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    show_latest(&fixture, &meta, 10, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.selected.timeline.older_error =
                Some(Failure::External("previous page failed".into()));
            view.navigate_to_marker(1, cx);
            let stale = view
                .selected
                .timeline
                .navigation
                .location
                .expect("first request");
            view.navigate_to_marker(3, cx);
            view.update(
                Update::HistoryLocation {
                    session: meta.id,
                    epoch: view.selected.epoch,
                    request: stale.request,
                    result: Ok(HistoryLocation {
                        window: body(&meta, 1, 2, 10),
                        after: Some(3),
                    }
                    .into()),
                },
                window,
                cx,
            );
            assert_eq!(
                view.selected
                    .timeline
                    .history
                    .as_ref()
                    .expect("unchanged body")
                    .events[0]
                    .cursor
                    .raw(),
                9
            );
            location(view, &meta, 3, 4, 10, window, cx);
            assert!(view.selected.timeline.navigation.detached);
            assert!(
                view.selected.timeline.older_error.is_none(),
                "new range clears previous paging failure"
            );
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(11),
                        EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                            bas::acp::ContentChunk::new("live suffix".into()),
                        )),
                    )
                    .into(),
                },
                window,
                cx,
            );
            assert_eq!(
                view.selected
                    .timeline
                    .history
                    .as_ref()
                    .expect("located slice")
                    .events
                    .last()
                    .expect("last")
                    .cursor
                    .raw(),
                4
            );
            view.load_next_history(cx);
            location(view, &meta, 5, 6, 11, window, cx);
            assert_eq!(
                view.selected
                    .timeline
                    .history
                    .as_ref()
                    .expect("joined")
                    .events
                    .iter()
                    .map(|event| event.cursor.raw())
                    .collect::<Vec<_>>(),
                vec![3, 4, 5, 6]
            );
            view.scroll_conversation_to_end(cx);
            assert!(!view.selected.timeline.navigation.detached);
            assert!(
                view.selected.timeline.history.is_none(),
                "reload latest instead of bridging a gap"
            );
            view.update(
                Update::HistoryWindow {
                    epoch: view.selected.epoch,
                    window: body(&meta, 9, 11, 11).into(),
                    cached: false,
                },
                window,
                cx,
            );
            assert_eq!(
                view.selected
                    .timeline
                    .history
                    .as_ref()
                    .expect("latest")
                    .events[0]
                    .cursor
                    .raw(),
                9
            );
            assert!(view.selected.timeline.list.is_following_tail());
        });
    })
    .expect("located and live histories");
}

#[gpui_kit::test]
fn switching_away_cancels_an_unloaded_jump_without_retaining_its_highlight(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let first = session();
    let other = session();
    show_latest(&fixture, &first, 10, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigate_to_marker(1, cx);
            assert_eq!(view.selected.timeline.navigation.selected, Some(1));
            view.sessions.entries.push(other.clone());
            view.select(other.id, window, cx);
            view.select(first.id, window, cx);
            assert!(view.selected.timeline.navigation.location.is_none());
            assert!(view.selected.timeline.navigation.selected.is_none());
            assert_eq!(
                view.selected
                    .timeline
                    .history
                    .as_ref()
                    .expect("cached real body")
                    .events
                    .first()
                    .expect("latest first")
                    .cursor
                    .raw(),
                9
            );
        });
    })
    .expect("cached pending jump");
}

#[gpui_kit::test]
fn alt_wheel_moves_between_complete_markers_without_pixel_scrolling(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    show_latest(&fixture, &meta, 20, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.navigate_to_marker(19, cx));
        window.render_frame(cx);
        let position = window.find("turn-marker-19").bounds().center();
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Lines(point(0., 1.)),
                modifiers: Modifiers {
                    alt: true,
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .navigation
                .location
                .expect("previous marker")
                .cursor,
            17
        );
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Lines(point(0., 1.)),
                modifiers: Modifiers {
                    alt: true,
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .navigation
                .location
                .expect("rapid previous")
                .cursor,
            15
        );
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Lines(point(0., -1.)),
                modifiers: Modifiers {
                    alt: true,
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .navigation
                .location
                .expect("next")
                .cursor,
            17
        );
        let list = fixture.view.read(cx).selected.timeline.list.clone();
        window.dispatch_event(
            ScrollWheelEvent {
                position: list.viewport_bounds().center(),
                delta: ScrollDelta::Lines(point(0., 1.)),
                modifiers: Modifiers {
                    alt: true,
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .selected
                .timeline
                .navigation
                .location
                .expect("Alt-wheel over body also navigates")
                .cursor,
            15
        );
    })
    .expect("Alt-wheel navigation");
}

#[gpui_kit::test]
fn overview_failure_and_late_responses_never_claim_a_complete_current_catalog(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let first = session();
    attach(&fixture, first.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let epoch = view.selected.epoch;
            let request = view
                .selected
                .timeline
                .navigation
                .overview_request
                .expect("initial request");
            view.update(
                Update::HistoryOverview {
                    session: first.id,
                    epoch,
                    request,
                    result: Err(Failure::from(tonic::Status::unimplemented(
                        "upgrade daemon for complete overview",
                    ))),
                },
                window,
                cx,
            );
            assert!(view.selected.timeline.navigation.overview.is_none());
            assert!(view.selected.timeline.navigation.overview_error.is_some());
            let second = session();
            view.sessions.entries.push(second.clone());
            view.select(second.id, window, cx);
            view.update(
                Update::HistoryOverview {
                    session: first.id,
                    epoch,
                    request,
                    result: Ok(overview(&first, 100).into()),
                },
                window,
                cx,
            );
            assert!(view.selected.timeline.navigation.overview.is_none());
        });
    })
    .expect("overview epoch isolation");
}
