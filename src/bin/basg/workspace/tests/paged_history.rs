use super::*;
use bas::service::HistoryWindow;
use gpui_kit::ListOffset;

fn page(meta: &SessionMeta, first: u64, last: u64) -> HistoryWindow {
    let events = (first..=last)
        .map(|cursor| {
            let chunk = bas::acp::ContentChunk::new(format!("Message {cursor}").into());
            let update = if cursor % 2 == 0 {
                bas::acp::SessionUpdate::AgentMessageChunk(chunk)
            } else {
                bas::acp::SessionUpdate::UserMessageChunk(chunk)
            };
            Event::new(meta.id, Cursor::from_raw(cursor), EventKind::Acp(update))
        })
        .collect();
    let mut meta = meta.clone();
    meta.revision = Cursor::from_raw(last);
    HistoryWindow {
        session: meta.into(),
        events,
        context: Vec::new(),
        before: (first > 1).then_some(first),
        cursor: last,
    }
}

fn tall_page(meta: &SessionMeta, first: u64, last: u64) -> HistoryWindow {
    let mut history = page(meta, first, last);
    for event in &mut history.events {
        if let EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(chunk)) = &mut event.kind {
            chunk.content = format!(
                "Message {}\n\n{}",
                event.cursor.raw(),
                "A paragraph.\n\n".repeat(30)
            )
            .into();
        }
    }
    history
}

fn show_page(
    view: &mut Workspace,
    history: HistoryWindow,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    view.update(
        Update::HistoryWindow {
            epoch: view.selected.epoch,
            window: history,
            cached: false,
        },
        window,
        cx,
    );
}

#[gpui_kit::test]
fn native_scrolling_survives_pages_resync_and_cached_switch(cx: &mut TestAppContext) {
    use gpui_kit::{PlatformInput, ScrollDelta, ScrollWheelEvent, point};
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            show_page(view, tall_page(&meta, 61, 100), window, cx);
        });
        window.render_frame(cx);
        let list = fixture.view.read(cx).selected.timeline.list.clone();
        assert!(list.is_following_tail(), "first open starts at latest");
        let last = fixture
            .view
            .read(cx)
            .selected
            .timeline
            .conversation
            .as_ref()
            .expect("conversation")
            .items
            .last()
            .expect("latest")
            .id();
        assert!(window.find(SharedString::from(last)).visible());
        window.dispatch_event(
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: list.viewport_bounds().center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(800.))),
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
        assert!(
            !list.is_following_tail(),
            "native upward input pauses follow"
        );
        let anchor = {
            let conversation = fixture
                .view
                .read(cx)
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("conversation");
            let row = &conversation.rows[list.logical_scroll_top().item_ix];
            conversation.items[row.items.start].id()
        };
        let anchor = SharedString::from(anchor);
        let bounds = window.find(anchor.clone()).bounds();
        assert!(window.find(anchor.clone()).visible());
        for before in [61, 41, 21] {
            fixture.view.update(cx, |view, cx| {
                view.load_older_history(cx);
                if before == 61 {
                    view.update(
                        Update::Event {
                            epoch: view.selected.epoch,
                            event: page(&meta, 101, 101).events.remove(0),
                        },
                        window,
                        cx,
                    );
                    show_page(view, tall_page(&meta, 81, 100), window, cx);
                    assert_eq!(
                        view.selected
                            .timeline
                            .history
                            .as_ref()
                            .expect("live history")
                            .cursor,
                        101
                    );
                }
            });
            window.render_frame(cx);
            assert_eq!(window.find(anchor.clone()).bounds(), bounds, "live append");
            fixture.view.update(cx, |view, cx| {
                view.update(
                    Update::OlderHistory {
                        epoch: view.selected.epoch,
                        before,
                        result: Ok(tall_page(&meta, before - 20, before - 1)),
                    },
                    window,
                    cx,
                );
            });
            for _ in 0..3 {
                window.render_frame(cx);
                assert_eq!(
                    window.find(anchor.clone()).bounds(),
                    bounds,
                    "page {before}"
                );
                assert!(!list.is_following_tail());
            }
        }
        fixture.view.update(cx, |view, cx| {
            show_page(view, tall_page(&meta, 81, 110), window, cx);
        });
        window.render_frame(cx);
        assert_eq!(
            window.find(anchor.clone()).bounds(),
            bounds,
            "safe-window resync"
        );
        assert!(!list.is_following_tail());
        // A response must not suspend the native wheel handler until repaint.
        fixture.view.update(cx, |view, cx| {
            show_page(view, tall_page(&meta, 81, 110), window, cx);
        });
        window.dispatch_event(
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: list.viewport_bounds().center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(40.))),
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
        let moved = window.find(anchor.clone()).bounds();
        assert_eq!(
            moved.top(),
            bounds.top() + px(40.),
            "native input during completion"
        );
        fixture.view.update(cx, |view, cx| {
            let other = session();
            view.sessions.entries.push(other.clone());
            view.select(other.id, window, cx);
        });
        window.render_frame(cx);
        fixture
            .view
            .update(cx, |view, cx| view.select(meta.id, window, cx));
        window.render_frame(cx);
        assert_eq!(window.find(anchor).bounds(), moved, "hot cache return");
        assert!(!list.is_following_tail());
        // Newly appended, unmeasured rows are discovered as the native list
        // scrolls; keep scrolling until the actual bottom, not its estimate.
        for _ in 0..20 {
            window.dispatch_event(
                PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position: list.viewport_bounds().center(),
                    delta: ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
                    ..Default::default()
                }),
                cx,
            );
            window.render_frame(cx);
            if list.is_following_tail() {
                break;
            }
        }
        assert!(
            list.is_following_tail(),
            "native return to bottom resumes follow"
        );
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: page(&meta, 111, 111).events.remove(0),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        let latest = fixture
            .view
            .read(cx)
            .selected
            .timeline
            .conversation
            .as_ref()
            .expect("conversation")
            .items
            .last()
            .expect("latest")
            .id();
        assert!(window.find(SharedString::from(latest)).visible());
        assert!(list.is_following_tail());
    })
    .expect("rendered native scrolling");
}

#[gpui_kit::test]
fn regrouping_before_reader_preserves_pixels_and_expansion(cx: &mut TestAppContext) {
    use gpui_kit::{PlatformInput, ScrollDelta, ScrollWheelEvent, point};
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    let tools = |first, last, range: std::ops::RangeInclusive<u64>| {
        let mut history = page(&meta, first, last);
        for event in &mut history.events {
            if range.contains(&event.cursor.raw()) {
                event.kind = EventKind::Acp(bas::acp::SessionUpdate::ToolCall(
                    bas::acp::ToolCall::new(format!("tool-{}", event.cursor.raw()), "Read source"),
                ));
            }
        }
        history
    };
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            show_page(view, tools(21, 60, 21..=23), window, cx);
            view.selected
                .timeline
                .expanded
                .insert("activity-tool-tool-21".into());
        });
        window.render_frame(cx);
        let list = fixture.view.read(cx).selected.timeline.list.clone();
        window.dispatch_event(
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: list.viewport_bounds().center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(800.))),
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
        let anchor = {
            let conversation = fixture
                .view
                .read(cx)
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("conversation");
            let row = &conversation.rows[list.logical_scroll_top().item_ix];
            conversation.items[row.items.start].id()
        };
        let anchor = SharedString::from(anchor);
        let bounds = window.find(anchor.clone()).bounds();
        fixture.view.update(cx, |view, cx| {
            view.load_older_history(cx);
            view.update(
                Update::OlderHistory {
                    epoch: view.selected.epoch,
                    before: 21,
                    result: Ok(tools(1, 20, 18..=20)),
                },
                window,
                cx,
            );
            assert!(
                view.selected
                    .timeline
                    .expanded
                    .contains("activity-tool-tool-18")
            );
        });
        for _ in 0..3 {
            window.render_frame(cx);
            assert_eq!(window.find(anchor.clone()).bounds(), bounds);
            assert!(!list.is_following_tail());
        }
        fixture.view.update(cx, |view, cx| {
            // A disconnected safe window may not contain the reading anchor.
            // Keep reading mode while requesting the missing range, never tail.
            show_page(view, page(&meta, 201, 240), window, cx);
            assert!(view.selected.timeline.restore.is_some());
        });
        window.render_frame(cx);
        assert!(
            !list.is_following_tail(),
            "missing anchor must not enable tail"
        );
        assert!(list.logical_scroll_top().item_ix < 5);
    })
    .expect("regrouping preceding activity");
}

#[gpui_kit::test]
#[ignore = "manual timing with a private BAS_HISTORY_BENCH_FILE HistoryWindow JSON"]
fn history_window_render_benchmark(cx: &mut TestAppContext) {
    let path = std::env::var_os("BAS_HISTORY_BENCH_FILE").expect("private fixture path");
    let bytes = std::fs::read(path).expect("read private fixture");
    let history: HistoryWindow = serde_json::from_slice(&bytes).expect("HistoryWindow JSON");
    let fixture = fixture(cx);
    attach(&fixture, history.session.meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let events = history.events.len();
        let context = history.context.len();
        let started = std::time::Instant::now();
        fixture.view.update(cx, |view, cx| show_page(view, history, window, cx));
        let applied = started.elapsed();
        let render_started = std::time::Instant::now();
        window.render_frame(cx);
        let first_frame = render_started.elapsed();
        let render_started = std::time::Instant::now();
        window.render_frame(cx);
        let second_frame = render_started.elapsed();
        let rows = fixture.view.read(cx).selected.timeline.list.item_count();
        eprintln!(
            "history benchmark: bytes={} events={events} context={context} rows={rows} apply_ms={:.3} first_frame_ms={:.3} second_frame_ms={:.3}",
            bytes.len(),
            applied.as_secs_f64() * 1000.,
            first_frame.as_secs_f64() * 1000.,
            second_frame.as_secs_f64() * 1000.,
        );
    }).expect("benchmark history render");
}

#[gpui_kit::test]
fn disk_cached_window_accepts_the_next_live_event(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.sessions.entries.push(meta.clone());
            view.select(meta.id, window, cx);
            view.update(
                Update::HistoryWindow {
                    epoch: view.selected.epoch,
                    window: page(&meta, 21, 40),
                    cached: true,
                },
                window,
                cx,
            );
            assert_eq!(
                view.selected
                    .timeline
                    .conversation
                    .as_ref()
                    .expect("cached")
                    .cursor,
                40
            );
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: page(&meta, 41, 41).events.remove(0),
                },
                window,
                cx,
            );
            let conversation = view.selected.timeline.conversation.as_ref().expect("live");
            assert_eq!(conversation.cursor, 41);
            assert_eq!(conversation.items.len(), 21);
            let history = view.selected.timeline.history.as_ref().expect("history");
            assert_eq!(history.cursor, 41);
            assert_eq!(history.events.len(), 21);
        });
    })
    .expect("disk cached history continues at the next live cursor");
}

#[gpui_kit::test]
fn prepend_keeps_anchor_live_events_and_requests_only_once(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            show_page(view, page(&meta, 21, 50), window, cx);
        });
        window.render_frame(cx);
        let anchor = fixture.view.update(cx, |view, cx| {
            view.selected.timeline.list.scroll_to(ListOffset {
                item_ix: 5,
                offset_in_item: px(4.),
            });
            view.load_older_history(cx);
            view.load_older_history(cx);
            view.selected
                .timeline
                .conversation
                .as_ref()
                .expect("recent page")
                .items[5]
                .id()
        });
        window.render_frame(cx);
        let offset = fixture
            .view
            .read(cx)
            .selected
            .timeline
            .list
            .logical_scroll_top();
        fixture.view.update(cx, |view, cx| {
            let event = page(&meta, 51, 51).events.remove(0);
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event,
                },
                window,
                cx,
            );
            show_page(view, page(&meta, 41, 60), window, cx);
            assert_eq!(view.selected.timeline.loading_before, Some(21));
            let mut older = page(&meta, 1, 20);
            older.cursor = 60;
            older.session.meta.revision = Cursor::from_raw(60);
            view.update(
                Update::OlderHistory {
                    epoch: view.selected.epoch,
                    before: 21,
                    result: Ok(older),
                },
                window,
                cx,
            );
            let history = view
                .selected
                .timeline
                .history
                .as_ref()
                .expect("merged pages");
            assert_eq!(history.cursor, 60);
            assert_eq!(history.events.len(), 60);
            assert_eq!(history.before, None);
            let conversation = view
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("rendered");
            assert_eq!(conversation.cursor, 60);
            assert_eq!(conversation.items[25].id(), anchor);
            let top = view.selected.timeline.list.logical_scroll_top();
            assert_eq!(top.item_ix, offset.item_ix + 20);
            assert_eq!(top.offset_in_item, offset.offset_in_item);
            assert!(!view.selected.timeline.list.is_following_tail());
        });
        window.render_frame(cx);
        assert!(window.try_find("older-history-status").is_none());
    })
    .expect("prepend current history");
    let mut older_requests = 0;
    while let Ok(command) = fixture.commands.try_recv() {
        if matches!(command, Command::OlderHistory { before: 21, .. }) {
            older_requests += 1;
        }
    }
    assert_eq!(older_requests, 1);
}

#[gpui_kit::test]
fn switching_back_restores_content_position_and_draft_then_resumes_cursor(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            show_page(view, page(&first, 21, 60), window, cx);
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("unfinished", window, cx));
            view.selected.timeline.list.scroll_to(ListOffset {
                item_ix: 6,
                offset_in_item: px(3.),
            });
            let id = view
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("timeline")
                .items[6]
                .id();
            view.selected.timeline.expanded.insert(id);
            view.sessions.entries.push(second.clone());
            view.select(second.id, window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("conversation-history-loading").visible());
        fixture.view.update(cx, |view, cx| {
            view.select(first.id, window, cx);
            assert!(view.selected.timeline.conversation.is_some());
            assert_eq!(view.selected.timeline.list.logical_scroll_top().item_ix, 6);
            assert_eq!(view.composer.input.read(cx).value(), "unfinished");
            let id = view
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("restored")
                .items[6]
                .id();
            assert!(view.selected.timeline.expanded.contains(&id));
            assert_eq!(view.selected.stream, StreamState::Loading);
        });
        window.render_frame(cx);
        assert!(window.try_find("conversation-history-loading").is_none());
    })
    .expect("switch cached timeline");
    let mut resumed = false;
    while let Ok(command) = fixture.commands.try_recv() {
        resumed |=
            matches!(command, Command::Resume { session, cursor: 60, .. } if session == first.id);
    }
    assert!(resumed, "hot history resumes without a snapshot request");
}

#[gpui_kit::test]
fn older_failure_retries_in_place_and_late_page_cannot_replace_another_session(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let first = session();
    let second = session();
    attach(&fixture, first.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let epoch = fixture.view.update(cx, |view, cx| {
            show_page(view, page(&first, 21, 40), window, cx);
            view.load_older_history(cx);
            view.update(
                Update::OlderHistory {
                    epoch: view.selected.epoch,
                    before: 21,
                    result: Err("older page offline".into()),
                },
                window,
                cx,
            );
            view.selected.epoch
        });
        window.render_frame(cx);
        assert!(window.find("older-history-retry").visible());
        window.click("older-history-error-copy", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("older page offline".into())
        );
        window.click("older-history-retry", cx);
        fixture.view.update(cx, |view, cx| {
            assert_eq!(view.selected.timeline.loading_before, Some(21));
            view.sessions.entries.push(second.clone());
            view.select(second.id, window, cx);
            view.update(
                Update::OlderHistory {
                    epoch,
                    before: 21,
                    result: Ok(page(&first, 1, 20)),
                },
                window,
                cx,
            );
            assert_eq!(view.selected.id, Some(second.id));
            assert!(view.selected.timeline.conversation.is_none());
        });
    })
    .expect("failure and selection race");
}

#[gpui_kit::test]
fn a_hundred_sessions_do_not_leave_a_hundred_timelines_in_memory(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for _ in 0..100 {
                let meta = session();
                view.sessions.entries.push(meta.clone());
                view.select(meta.id, window, cx);
                show_page(view, page(&meta, 1, 8), window, cx);
            }
            assert!(view.sessions.history.hot_len() <= 4);
            assert_eq!(
                view.selected
                    .timeline
                    .history
                    .as_ref()
                    .expect("active")
                    .events
                    .len(),
                8
            );
        });
    })
    .expect("bounded hot set");
}
