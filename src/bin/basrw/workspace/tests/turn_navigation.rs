use basr::acp::{ContentChunk, SessionUpdate, ToolCall, ToolCallStatus};
use basr::model::{Role, StopReason, TurnId};
use std::time::Duration;

use super::*;

struct TurnTargets {
    user_items: Vec<usize>,
    user_rows: Vec<usize>,
    item_count: usize,
}

fn add_turns(fixture: &Fixture, window: &mut Window, cx: &mut App) -> TurnTargets {
    fixture.view.update(cx, |view, cx| {
        let session = view.active.expect("active session");
        let mut cursor = view.conversation.as_ref().expect("conversation").cursor;
        for number in 1..=3 {
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
                    format!("第{number}轮回复。\n\n{}", "详细内容。\n\n".repeat(80)).into(),
                ))),
                EventKind::TurnFinished {
                    turn,
                    result: Ok(basr::acp::PromptResponse::new(StopReason::EndTurn)),
                },
            ];
            for kind in events {
                cursor += 1;
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(session, Cursor::from_raw(cursor), kind),
                    },
                    window,
                    cx,
                );
            }
        }

        let conversation = view.conversation.as_ref().expect("conversation");
        let user_items: Vec<_> = conversation
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                matches!(item, Item::Message { message, .. } if message.role == Role::User)
                    .then_some(index)
            })
            .collect();
        assert_eq!(user_items.len(), 3);
        assert!(
            conversation.rows.len() < conversation.items.len(),
            "thoughts and tools should share activity rows"
        );
        let user_rows = user_items
            .iter()
            .map(|&index| conversation.row_for_item(index).expect("user row"))
            .collect();
        view.composer_expanded = false;
        TurnTargets {
            user_items,
            user_rows,
            item_count: conversation.items.len(),
        }
    })
}

#[gpui_kit::test]
fn turn_markers_follow_user_messages_and_jump_to_grouped_rows(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}

    cx.update_window(fixture.window.into(), |_, window, cx| {
        let targets = add_turns(&fixture, window, cx);
        assert_ne!(targets.user_items[1], targets.user_rows[1]);
        window.render_frame(cx);

        let timeline = window.find("conversation-timeline").bounds();
        let body = window.find("thread-body").bounds();
        let rail = window.find("turn-navigation");
        assert!(rail.visible());
        let rail = rail.bounds();
        assert_eq!(rail.left(), timeline.right());
        assert_eq!(rail.right(), body.right());
        assert_eq!(rail.size.width, px(24.));
        let mut prompt_centers = Vec::new();
        for index in 0..targets.item_count {
            if let Some(marker) = window.try_find(format!("turn-marker-{index}")) {
                assert!(marker.visible(), "prompt marker {index} should be visible");
                assert_eq!(marker.role(), Some(gpui_kit::Role::Button));
                let bounds = marker.bounds();
                assert_eq!(bounds.left(), rail.left() + px(1.));
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
        assert_eq!(end.left(), rail.left() + px(1.));
        assert_eq!(end.right(), rail.right());
        assert_eq!(end.size.height, px(12.));
        assert!(end.bottom() <= rail.bottom());
        assert!(rail.bottom() - end.bottom() <= px(1.));
        assert!(prompt_centers.iter().all(|&y| end.center().y > y));
        let end_line = window.find("turn-marker-line-end");
        assert!(end_line.visible());
        let end_line = end_line.bounds();
        assert_eq!(end_line.size.width, px(18.));
        assert!(end_line.left() >= end.left() && end_line.right() <= end.right());

        fixture.view.read(cx).list.set_follow_mode(FollowMode::Tail);
        window.render_frame(cx);
        window.click(format!("turn-marker-{}", targets.user_items[1]), cx);
        let list = fixture.view.read(cx).list.clone();
        let scroll = list.logical_scroll_top();
        assert_eq!(scroll.item_ix, targets.user_rows[1]);
        assert_eq!(scroll.offset_in_item, px(0.));
        assert!(!list.is_following_tail());

        window.click(format!("turn-marker-{}", targets.user_items[0]), cx);
        let scroll = list.logical_scroll_top();
        assert_eq!(scroll.item_ix, targets.user_rows[0]);
        assert_eq!(scroll.offset_in_item, px(0.));
        window.hover(format!("turn-marker-{}", targets.user_items[0]), cx);
    })
    .expect("turn markers");

    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("bounded-tooltip-content").visible());
    })
    .expect("prompt hint");
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn end_marker_and_ctrl_end_resume_following_the_timeline(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}

    cx.update_window(fixture.window.into(), |_, window, cx| {
        let targets = add_turns(&fixture, window, cx);
        window.render_frame(cx);
        let first = format!("turn-marker-{}", targets.user_items[0]);
        let list = fixture.view.read(cx).list.clone();

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
            view.search.update(cx, |input, cx| input.focus(window, cx))
        });
        window.press("ctrl-end", cx);
        assert!(list.is_following_tail(), "shortcut works from search");
    })
    .expect("end navigation");
    assert!(fixture.commands.try_recv().is_err());
}
