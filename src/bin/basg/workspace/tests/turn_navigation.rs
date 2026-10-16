use bas::acp::{ContentChunk, SessionUpdate, ToolCall, ToolCallStatus};
use bas::model::{Role, StopReason, TurnId};
use std::time::Duration;

use super::*;

struct TurnTargets {
    user_items: Vec<usize>,
    user_rows: Vec<usize>,
    item_count: usize,
}

fn add_turns(fixture: &Fixture, count: usize, window: &mut Window, cx: &mut App) -> TurnTargets {
    fixture.view.update(cx, |view, cx| {
        let session = view.selected.id.expect("active session");
        let mut cursor = view
            .selected
            .timeline
            .conversation
            .as_ref()
            .expect("conversation")
            .cursor;
        for number in 1..=count {
            let turn = TurnId::new();
            let events = [
                EventKind::TurnStarted { turn },
                EventKind::Acp(SessionUpdate::UserMessageChunk(ContentChunk::new(
                    format!("第{number}轮请求：请检查这一段内容").into(),
                ))),
                EventKind::Acp(SessionUpdate::AgentThoughtChunk(ContentChunk::new(
                    "正在检查".into(),
                ))),
                EventKind::Acp(SessionUpdate::ToolCall(
                    ToolCall::new(format!("read-{number}"), "读取文件")
                        .status(ToolCallStatus::Completed),
                )),
                EventKind::Acp(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    format!(
                        "第{number}轮回复。\n\n{}",
                        "详细内容。\n\n".repeat(if number == 2 || number == count {
                            80
                        } else {
                            2
                        })
                    )
                    .into(),
                ))),
                EventKind::TurnFinished {
                    turn,
                    result: Ok(bas::acp::PromptResponse::new(StopReason::EndTurn)),
                },
            ];
            for kind in events {
                cursor += 1;
                view.update(
                    Update::Event {
                        epoch: view.selected.epoch,
                        event: Event::new(session, Cursor::from_raw(cursor), kind),
                    },
                    window,
                    cx,
                );
            }
        }

        let conversation = view
            .selected
            .timeline
            .conversation
            .as_ref()
            .expect("conversation");
        let user_items: Vec<_> = conversation
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                matches!(item, Item::Message { message, .. } if message.role == Role::User)
                    .then_some(index)
            })
            .collect();
        assert_eq!(user_items.len(), count);
        assert!(
            conversation.rows.len() < conversation.items.len(),
            "thoughts and tools should share activity rows"
        );
        let user_rows = user_items
            .iter()
            .map(|&index| conversation.row_for_item(index).expect("user row"))
            .collect();
        view.composer.expanded = false;
        TurnTargets {
            user_items,
            user_rows,
            item_count: conversation.items.len(),
        }
    })
}

#[gpui_kit::test]
fn turn_shortcuts_stay_together_and_jump_to_grouped_rows(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}

    cx.update_window(fixture.window.into(), |_, window, cx| {
        let targets = add_turns(&fixture, 3, window, cx);
        assert_ne!(targets.user_items[1], targets.user_rows[1]);
        fixture
            .view
            .read(cx)
            .selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        window.render_frame(cx);

        let body = window.find("thread-body").bounds();
        let rail = window.find("turn-navigation");
        assert!(rail.visible());
        let rail = rail.bounds();
        assert_eq!(
            rail.left(),
            window.find("timeline-navigation-anchor").bounds().right()
        );
        assert!(rail.right() <= body.right());
        assert_eq!(
            window.find("timeline-scrollbar").bounds().right(),
            body.right()
        );
        assert_eq!(rail.size.width, px(24.));
        let mut prompt_centers = Vec::new();
        for index in 0..targets.item_count {
            if let Some(marker) = window.try_find(format!("turn-marker-{index}")) {
                assert!(marker.visible(), "prompt marker {index} should be visible");
                assert_eq!(marker.role(), Some(gpui_kit::Role::Button));
                let bounds = marker.bounds();
                assert_eq!(bounds.left(), rail.left());
                assert_eq!(bounds.right(), rail.right());
                assert_eq!(bounds.size.height, px(12.));
                assert!(bounds.top() >= rail.top() && bounds.bottom() <= rail.bottom());
                let line = window.find(format!("turn-marker-line-{index}"));
                assert!(line.visible());
                let line = line.bounds();
                assert_eq!(line.size.width, px(14.));
                assert!(line.left() >= bounds.left() && line.right() <= bounds.right());
                prompt_centers.push(bounds.center().y);
            }
        }
        assert_eq!(prompt_centers.len(), targets.user_items.len());
        for pair in prompt_centers.windows(2) {
            assert_eq!(pair[1] - pair[0], px(12.), "shortcuts have fixed spacing");
        }
        let group = window.find("turn-shortcuts").bounds();
        assert_eq!(group.size.height, px(48.));
        assert_eq!(group.center().y, rail.center().y);
        assert_eq!(
            window
                .find(format!("turn-marker-{}", targets.user_items[0]))
                .label(),
            Some("第1轮请求：请检查这一段内容")
        );
        let end = window.find("turn-marker-end");
        assert!(end.visible());
        assert_eq!(end.role(), Some(gpui_kit::Role::Button));
        assert_eq!(end.label(), Some("跳到对话末尾"));
        let end = end.bounds();
        assert_eq!(end.left(), rail.left());
        assert_eq!(end.right(), rail.right());
        assert_eq!(end.size.height, px(12.));
        assert!(end.bottom() <= rail.bottom());
        assert_eq!(end.bottom(), group.bottom());
        assert_eq!(
            end.center().y - *prompt_centers.last().expect("last shortcut"),
            px(12.)
        );
        assert!(prompt_centers.iter().all(|&y| end.center().y > y));
        let end_line = window.find("turn-marker-line-end");
        assert!(end_line.visible());
        let end_line = end_line.bounds();
        assert_eq!(end_line.size.width, px(18.));
        assert!(end_line.left() >= end.left() && end_line.right() <= end.right());

        fixture
            .view
            .read(cx)
            .selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        window.render_frame(cx);
        window.click(format!("turn-marker-{}", targets.user_items[1]), cx);
        let list = fixture.view.read(cx).selected.timeline.list.clone();
        let scroll = list.logical_scroll_top();
        assert_eq!(scroll.item_ix, targets.user_rows[1]);
        assert_eq!(scroll.offset_in_item, px(0.));
        assert!(!list.is_following_tail());

        window.click(format!("turn-marker-{}", targets.user_items[0]), cx);
        let scroll = list.logical_scroll_top();
        assert_eq!(scroll.item_ix, targets.user_rows[0]);
        assert_eq!(scroll.offset_in_item, px(0.));
        for height in [600., 1000.] {
            window.resize(size(px(920.), px(height)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            let group = window.find("turn-shortcuts").bounds();
            let rail = window.find("turn-navigation").bounds();
            assert_eq!(group.size.height, px(48.));
            assert_eq!(group.center().y, rail.center().y);
            let first = window
                .find(format!("turn-marker-{}", targets.user_items[0]))
                .bounds();
            let next = window
                .find(format!("turn-marker-{}", targets.user_items[1]))
                .bounds();
            assert_eq!(
                next.top() - first.top(),
                px(12.),
                "resizing keeps the group compact"
            );
        }
        window.hover(format!("turn-marker-{}", targets.user_items[0]), cx);
    })
    .expect("turn markers");

    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("bounded-tooltip-content").visible());
        assert_eq!(
            window.find("bounded-tooltip-content").label(),
            Some("第1轮请求：请检查这一段内容")
        );
    })
    .expect("prompt hint");
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn many_turn_shortcuts_scroll_independently_and_keep_the_end_reachable(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    let targets = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            add_turns(&fixture, 80, window, cx)
        })
        .expect("long conversation");
    cx.run_until_parked();
    for compact in [false, true] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.window.compact = compact;
                view.window.output_open = true;
                view.resize_window(window, cx);
                view.selected
                    .timeline
                    .list
                    .scroll_to(gpui_kit::ListOffset::default());
            });
            window.resize(size(
                px(if compact { 420. } else { 920. }),
                px(if compact { 440. } else { 700. }),
            ));
            window.bounds_changed(cx);
            window.render_frame(cx);
            let session = fixture.view.read(cx).selected.id.expect("active session");
            let prompts = format!("turn-prompts-{session}");
            window.scroll(
                prompts.clone(),
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(5000.))),
                cx,
            );
            let rail = window.find("turn-navigation").bounds();
            let group = window.find("turn-shortcuts").bounds();
            let end = window.find("turn-marker-end").bounds();
            let viewport = window.find(prompts.clone()).bounds();
            assert!(
                group.top() >= rail.top() && group.bottom() <= rail.bottom(),
                "long groups fit the viewport"
            );
            assert!(viewport.bottom() <= end.top());
            assert!(viewport.size.height <= px(240.));
            assert!(group.size.height <= px(252.));
            assert_eq!(group.center().y, rail.center().y);
            assert_eq!(end.size.height, px(12.));
            let first = format!("turn-marker-{}", targets.user_items[0]);
            let last = format!(
                "turn-marker-{}",
                targets.user_items.last().expect("last turn")
            );
            assert!(window.find(first).visible());
            assert!(
                !window.find(last.clone()).visible(),
                "offscreen shortcuts are clipped"
            );
            let before = fixture
                .view
                .read(cx)
                .selected
                .timeline
                .list
                .logical_scroll_top();
            window.scroll(
                prompts.clone(),
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-5000.))),
                cx,
            );
            window.scroll(
                prompts,
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-5000.))),
                cx,
            );
            let after = fixture
                .view
                .read(cx)
                .selected
                .timeline
                .list
                .logical_scroll_top();
            assert_eq!(
                after.item_ix, before.item_ix,
                "scrolling shortcuts, including at their boundary, leaves the conversation in place"
            );
            assert_eq!(after.offset_in_item, before.offset_in_item);
            assert!(window.find(last.clone()).visible());
            assert!(window.find("turn-marker-end").visible());
            window.click(last, cx);
            let list = fixture.view.read(cx).selected.timeline.list.clone();
            let target = *targets.user_rows.last().expect("last user row");
            assert!(list.logical_scroll_top().item_ix <= target);
            let bounds = list.bounds_for_item(target).expect("last user is laid out");
            assert!(
                bounds.bottom() > list.viewport_bounds().top()
                    && bounds.top() < list.viewport_bounds().bottom(),
                "last user is visible after jumping"
            );
            assert!(!list.is_following_tail());
            window.click("turn-marker-end", cx);
            assert!(list.is_following_tail());
        })
        .expect("shortcuts in full and compact layouts");
    }
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn keyboard_pages_and_turns_navigate_without_editing_the_composer(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let targets = add_turns(&fixture, 4, window, cx);
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("keep this draft", window, cx);
            });
            view.focus_input(window, cx);
        });
        window.render_frame(cx);
        let list = fixture.view.read(cx).selected.timeline.list.clone();
        window.press("ctrl-home", cx);
        assert_eq!(list.logical_scroll_top().item_ix, 0);
        assert_eq!(list.logical_scroll_top().offset_in_item, px(0.));
        assert!(!list.is_following_tail());
        for &row in targets.user_rows.iter().skip(1) {
            window.press("ctrl-pagedown", cx);
            assert_eq!(list.logical_scroll_top().item_ix, row);
            assert_eq!(list.logical_scroll_top().offset_in_item, px(0.));
        }
        for &row in targets.user_rows.iter().rev().skip(1) {
            window.press("ctrl-pageup", cx);
            assert_eq!(list.logical_scroll_top().item_ix, row);
        }
        window.press("ctrl-pageup", cx);
        assert_eq!(list.logical_scroll_top().item_ix, targets.user_rows[0]);
        window.press("ctrl-pagedown", cx);
        window.render_frame(cx);
        window.press("pagedown", cx);
        window.render_frame(cx);
        let before = list.logical_scroll_top();
        let height = list.viewport_bounds().size.height;
        window.press("pagedown", cx);
        window.render_frame(cx);
        let after = list.logical_scroll_top();
        assert_eq!(
            before.item_ix, after.item_ix,
            "long reply stays in the same row"
        );
        assert!((f32::from(after.offset_in_item - before.offset_in_item - height)).abs() < 1.);
        window.press("pageup", cx);
        window.render_frame(cx);
        assert_eq!(list.logical_scroll_top().item_ix, before.item_ix);
        assert_eq!(
            list.logical_scroll_top().offset_in_item,
            before.offset_in_item
        );
        window.press("ctrl-end", cx);
        assert!(list.is_following_tail());
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "keep this draft"
        );
        fixture.view.update(cx, |view, cx| {
            view.navigation
                .search
                .update(cx, |input, cx| input.focus(window, cx));
        });
        window.press("ctrl-home", cx);
        assert_eq!(list.logical_scroll_top().item_ix, 0);
        assert_eq!(list.logical_scroll_top().offset_in_item, px(0.));
    })
    .expect("keyboard timeline navigation");
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn turn_navigation_tracks_browsing_and_stays_beside_wide_content(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let targets = add_turns(&fixture, 4, window, cx);
        window.resize(size(px(1800.), px(850.)));
        window.bounds_changed(cx);
        fixture
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        window.render_frame(cx);
        let anchor = window.find("timeline-navigation-anchor").bounds();
        assert_eq!(anchor.size.width, px(chat::CONTENT_WIDTH));
        assert_eq!(
            window.find("turn-navigation").bounds().left(),
            anchor.right()
        );
        assert!(window.find("timeline-scrollbar").bounds().left() > anchor.right() + px(24.));
        window.press("ctrl-home", cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find(format!("turn-marker-line-{}", targets.user_items[0]))
                .bounds()
                .size
                .width,
            px(20.)
        );
        window.press("ctrl-pagedown", cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find(format!("turn-marker-line-{}", targets.user_items[0]))
                .bounds()
                .size
                .width,
            px(14.)
        );
        assert_eq!(
            window
                .find(format!("turn-marker-line-{}", targets.user_items[1]))
                .bounds()
                .size
                .width,
            px(20.)
        );
        window.press("ctrl-end", cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find(format!("turn-marker-line-{}", targets.user_items[1]))
                .bounds()
                .size
                .width,
            px(14.)
        );
        window.press("ctrl-home", cx);
        window.scroll(
            "conversation-timeline",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-600.))),
            cx,
        );
        let list = &fixture.view.read(cx).selected.timeline.list;
        let top = list.logical_scroll_top().item_ix;
        assert!(
            top > targets.user_rows[0],
            "the overlay must not block wheel scrolling"
        );
        assert!(!list.is_following_tail());
        let active = targets
            .user_rows
            .iter()
            .rposition(|row| *row <= top)
            .expect("visible turn");
        assert_eq!(
            window
                .find(format!("turn-marker-line-{}", targets.user_items[active]))
                .bounds()
                .size
                .width,
            px(20.),
            "wheel scrolling updates the highlighted turn",
        );
    })
    .expect("content-local navigation");
}

#[gpui_kit::test]
fn end_marker_and_ctrl_end_resume_following_the_timeline(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}

    cx.update_window(fixture.window.into(), |_, window, cx| {
        let targets = add_turns(&fixture, 3, window, cx);
        window.render_frame(cx);
        let first = format!("turn-marker-{}", targets.user_items[0]);
        let list = fixture.view.read(cx).selected.timeline.list.clone();

        window.click(first.clone(), cx);
        assert!(!list.is_following_tail());
        let start = list.logical_scroll_top().item_ix;
        window.click("turn-marker-end", cx);
        assert!(list.is_following_tail());
        assert!(list.logical_scroll_top().item_ix > start);

        window.click(first.clone(), cx);
        window.press("ctrl-end", cx);
        assert!(
            list.is_following_tail(),
            "shortcut works after mouse navigation"
        );

        window.click(first, cx);
        fixture
            .view
            .update(cx, |view, cx| view.focus_input(window, cx));
        window.press("ctrl-end", cx);
        assert!(list.is_following_tail(), "shortcut works from the composer");

        window.click(format!("turn-marker-{}", targets.user_items[0]), cx);
        fixture.view.update(cx, |view, cx| {
            view.navigation
                .search
                .update(cx, |input, cx| input.focus(window, cx))
        });
        window.press("ctrl-end", cx);
        assert!(list.is_following_tail(), "shortcut works from search");
    })
    .expect("end navigation");
    assert!(fixture.commands.try_recv().is_err());
}
