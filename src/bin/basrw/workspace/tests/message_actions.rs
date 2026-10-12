use basr::acp::{ContentChunk, SessionUpdate, ToolCall, ToolCallStatus};
use basr::model::{StopReason, TurnId};
use gpui_kit::InputEvent as _;
use std::rc::Rc;

use super::*;

fn user(text: &str) -> EventKind {
    EventKind::Acp(SessionUpdate::UserMessageChunk(ContentChunk::new(
        text.into(),
    )))
}

fn agent(text: &str) -> EventKind {
    EventKind::Acp(SessionUpdate::AgentMessageChunk(ContentChunk::new(
        text.into(),
    )))
}

fn thought() -> EventKind {
    EventKind::Acp(SessionUpdate::AgentThoughtChunk(ContentChunk::new(
        "不应复制的思考".into(),
    )))
}

fn tool(id: &str) -> EventKind {
    EventKind::Acp(SessionUpdate::ToolCall(
        ToolCall::new(id.to_owned(), "不应复制的工具输出").status(ToolCallStatus::Completed),
    ))
}

fn finished(turn: TurnId) -> EventKind {
    EventKind::TurnFinished {
        turn,
        result: Ok(basr::acp::PromptResponse::new(StopReason::EndTurn)),
    }
}

fn deliver(
    fixture: &Fixture,
    kinds: impl IntoIterator<Item = EventKind>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<String> {
    fixture.view.update(cx, |view, cx| {
        let session = view.active.expect("test session");
        let cursor = view.conversation.as_ref().expect("conversation").cursor;
        for (offset, kind) in kinds.into_iter().enumerate() {
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(session, Cursor::from_raw(cursor + offset as u64 + 1), kind),
                },
                window,
                cx,
            );
        }
        view.conversation
            .as_ref()
            .expect("conversation")
            .items
            .iter()
            .map(Item::id)
            .collect()
    })
}

fn assert_global_navigation_group(window: &Window, compact: bool) {
    let host = window
        .find(if compact {
            "compact-header"
        } else {
            "conversation-status"
        })
        .bounds();
    let group = window.find("conversation-navigation").bounds();
    let top = window.find("scroll-to-top").bounds();
    let bottom = window.find("scroll-to-bottom").bounds();
    assert!(top.right() <= bottom.left());
    assert!(bottom.left() - top.right() <= px(8.));
    assert_eq!(top.top(), bottom.top());
    assert_eq!(top.bottom(), bottom.bottom());
    assert!(group.left() <= top.left() && bottom.right() <= group.right());
    assert!(host.left() <= group.left() && group.right() <= host.right());
    assert!(host.top() <= group.top() && group.bottom() <= host.bottom());
    if !compact {
        assert_eq!(group.right(), host.right());
    }
}

struct ActionStyleProbe {
    color: Rc<Cell<Option<gpui_kit::Hsla>>>,
    disabled_color: Rc<Cell<Option<gpui_kit::Hsla>>>,
    disabled_clicks: Rc<Cell<usize>>,
}

impl Render for ActionStyleProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = self.color.clone();
        let disabled_color = self.disabled_color.clone();
        let disabled_clicks = self.disabled_clicks.clone();
        div()
            .p_4()
            .flex()
            .gap_2()
            .child(
                crate::workspace::message_actions::action_button(
                    "secondary-action",
                    IconName::Copy,
                    cx,
                )
                .on_click(|_, _, _| {})
                .child(
                    gpui_kit::canvas(
                        move |_, window, _| color.set(Some(window.text_style().color)),
                        |_, _, _, _| {},
                    )
                    .size(px(1.)),
                ),
            )
            .child(
                crate::workspace::message_actions::action_button(
                    "disabled-secondary-action",
                    IconName::ArrowUp,
                    cx,
                )
                .disabled(true)
                .on_click(move |_, _, _| disabled_clicks.set(disabled_clicks.get() + 1))
                .child(
                    gpui_kit::canvas(
                        move |_, window, _| {
                            disabled_color.set(Some(window.text_style().color));
                        },
                        |_, _, _, _| {},
                    )
                    .size(px(1.)),
                ),
            )
    }
}

#[gpui_kit::test]
fn secondary_action_icons_are_subtle_until_hovered_and_keep_disabled_feedback(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init_default(cx);
    });
    let color = Rc::new(Cell::new(None));
    let disabled_color = Rc::new(Cell::new(None));
    let disabled_clicks = Rc::new(Cell::new(0));
    let observed = color.clone();
    let disabled_observed = disabled_color.clone();
    let disabled_clicks_observed = disabled_clicks.clone();
    let window = cx.open_window(size(px(320.), px(180.)), |window, cx| {
        let probe = cx.new(|_| ActionStyleProbe {
            color: observed,
            disabled_color: disabled_observed,
            disabled_clicks: disabled_clicks_observed,
        });
        Root::new(probe, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            Theme::change(mode, Some(window), cx);
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: gpui_kit::point(px(300.), px(160.)),
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let idle = color
                .get()
                .expect("the rendered action's inherited foreground");
            let muted = cx.theme().muted_foreground;
            assert_eq!((idle.h, idle.s, idle.l), (muted.h, muted.s, muted.l));
            assert!(
                idle.a < muted.a,
                "actions have lower emphasis than auxiliary text"
            );
            assert_ne!(idle, cx.theme().foreground);
            let disabled_idle = disabled_color
                .get()
                .expect("the disabled action's inherited foreground");
            assert_eq!(disabled_idle, muted.opacity(0.5));
            window.hover("secondary-action", cx);
            assert_eq!(
                color.get(),
                Some(cx.theme().foreground),
                "hover restores the bright icon foreground"
            );
            window.hover("disabled-secondary-action", cx);
            assert_eq!(
                color.get(),
                Some(idle),
                "leaving an action restores its muted color"
            );
            assert_eq!(
                disabled_color.get(),
                Some(disabled_idle),
                "disabled icons remain muted on hover"
            );
            window.click("disabled-secondary-action", cx);
            assert_eq!(
                disabled_clicks.get(),
                0,
                "disabled actions ignore activation"
            );
        }
    })
    .expect("action colors in both themes");
}

#[gpui_kit::test]
fn completed_response_actions_copy_markdown_and_navigate_grouped_timeline_rows(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let first = TurnId::new();
        let second = TurnId::new();
        let second_answer = format!("第二轮结论\n\n{}", "长回复的详细说明。\n\n".repeat(80));
        let ids = deliver(
            &fixture,
            [
                EventKind::TurnStarted { turn: first },
                user("第一轮用户请求"),
                thought(),
                tool("first-read"),
                agent("**先检查**"),
                tool("first-verify"),
                agent("**第一轮结论**"),
                finished(first),
                EventKind::TurnStarted { turn: second },
                user("第二轮用户请求"),
                thought(),
                tool("second-read"),
                agent(&second_answer),
                finished(second),
            ],
            window,
            cx,
        );
        let second_user_row = fixture
            .view
            .read(cx)
            .conversation
            .as_ref()
            .expect("conversation")
            .row_for_item(6)
            .expect("second user row");
        assert_ne!(second_user_row, 6, "activity items use a grouped row");
        for compact in [false, true] {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.output_open = true;
                view.composer_expanded = false;
                view.resize_window(window, cx);
                view.list.set_follow_mode(FollowMode::Normal);
                view.list.scroll_to(gpui_kit::ListOffset::default());
            });
            window.resize(size(px(1120.), px(1000.)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            assert!(
                window
                    .try_find(format!("message-actions-{}", ids[3]))
                    .is_none()
            );
            assert!(window.find(format!("message-actions-{}", ids[5])).visible());
            assert!(
                window
                    .try_find(format!("scroll-to-top-{}", ids[5]))
                    .is_none()
            );
            assert!(window.try_find(format!("copy-{}", ids[5])).is_none());
            assert!(window.try_find(format!("copy-{}", ids[6])).is_some());

            window.hover(format!("message-actions-{}", ids[5]), cx);
            window.click(format!("copy-response-{}", ids[5]), cx);
            assert_eq!(
                cx.read_from_clipboard().expect("copied response").text(),
                Some("**先检查**\n\n**第一轮结论**".into())
            );
            fixture.view.read(cx).list.set_follow_mode(FollowMode::Tail);
            window.render_frame(cx);
            assert!(window.find(format!("message-actions-{}", ids[9])).visible());
            assert!(
                window
                    .try_find(format!("scroll-to-top-{}", ids[9]))
                    .is_none()
            );
            assert_global_navigation_group(window, compact);
            let list = &fixture.view.read(cx).list;
            assert!(
                window.find(ids[9].clone()).bounds().size.height
                    > list.viewport_bounds().size.height,
                "the navigation target needs overflowing content below it"
            );
            window.hover(format!("message-actions-{}", ids[9]), cx);
            window.click(format!("copy-response-{}", ids[9]), cx);
            assert_eq!(
                cx.read_from_clipboard().expect("copied response").text(),
                Some(second_answer.clone())
            );
            window.click(format!("scroll-to-user-{}", ids[9]), cx);
            let scroll = fixture.view.read(cx).list.logical_scroll_top();
            assert_eq!(scroll.item_ix, second_user_row);
            assert_eq!(scroll.offset_in_item, px(0.));
            assert!(!fixture.view.read(cx).list.is_following_tail());
            window.render_frame(cx);
            assert_eq!(
                fixture.view.read(cx).list.logical_scroll_top().item_ix,
                second_user_row
            );

            fixture.view.read(cx).list.set_follow_mode(FollowMode::Tail);
            window.click("scroll-to-top", cx);
            let top = fixture.view.read(cx).list.logical_scroll_top();
            assert_eq!(top.item_ix, 0);
            assert_eq!(top.offset_in_item, px(0.));
            window.click(format!("scroll-to-user-{}", ids[5]), cx);
            let top = fixture.view.read(cx).list.logical_scroll_top();
            assert_eq!(top.item_ix, 0);
            assert_eq!(top.offset_in_item, px(0.));
            window.click("scroll-to-bottom", cx);
            assert!(fixture.view.read(cx).list.is_following_tail());
        }
    })
    .expect("response actions in both window modes");
    assert!(
        fixture.commands.try_recv().is_err(),
        "navigation sends no agent command"
    );
}

#[gpui_kit::test]
fn only_the_active_response_hides_actions_while_working_or_awaiting_input(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let first = TurnId::new();
        let second = TurnId::new();
        let ids = deliver(
            &fixture,
            [
                EventKind::TurnStarted { turn: first },
                user("已完成的请求"),
                agent("已完成的回复"),
                finished(first),
                EventKind::TurnStarted { turn: second },
                user("正在执行的请求"),
                agent("正在输出的回复"),
            ],
            window,
            cx,
        );
        window.render_frame(cx);
        assert!(window.find(format!("message-actions-{}", ids[1])).visible());
        assert!(window.try_find(format!("message-actions-{}", ids[3])).is_none());
        deliver(&fixture, [finished(second)], window, cx);
        window.render_frame(cx);
        assert!(window.find(format!("message-actions-{}", ids[3])).visible());
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Permissions {
                    epoch: view.epoch,
                    pending: vec![basr::model::PendingPermission {
                        request_id: "response-permission".into(),
                        request: basr::acp::RequestPermissionRequest::new(
                            meta.id.to_string(),
                            basr::acp::ToolCallUpdate::new("write", basr::acp::ToolCallUpdateFields::new()),
                            Vec::new(),
                        ),
                    }],
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert!(window.find(format!("message-actions-{}", ids[1])).visible());
        assert!(window.try_find(format!("message-actions-{}", ids[3])).is_none());
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Permissions { epoch: view.epoch, pending: Vec::new() }, window, cx);
            view.update(
                Update::Elicitations {
                    epoch: view.epoch,
                    pending: vec![pb::PendingElicitation {
                        request_id: "response-question".into(),
                        request: Some(pb::AcpPayload {
                            json: serde_json::to_vec(&serde_json::json!({
                                "sessionId": meta.id.to_string(),
                                "mode": "form",
                                "message": "请选择",
                                "requestedSchema": {"type": "object", "properties": {"name": {"type": "string"}}}
                            })).expect("elicitation payload"),
                        }),
                        answered: false,
                    }],
                    completed_ids: Vec::new(),
                },
                window,
                cx,
            );
            assert!(!view.elicitation_panels.is_empty());
        });
        window.render_frame(cx);
        assert!(window.find(format!("message-actions-{}", ids[1])).visible());
        assert!(window.try_find(format!("message-actions-{}", ids[3])).is_none());
    })
    .expect("active response waits for completion and user input");
}

#[gpui_kit::test]
fn tool_and_thought_tail_actions_stay_outside_collapsed_activity_and_target_the_user(
    cx: &mut TestAppContext,
) {
    for tail_kind in ["tool", "thought", "thought-only"] {
        let fixture = fixture(cx);
        let meta = session();
        attach(&fixture, meta.clone(), cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            let first = TurnId::new();
            let second = TurnId::new();
            let answer = format!("**第二轮结论**\n\n{}", "长回复的详细说明。\n\n".repeat(80));
            let mut events = vec![
                EventKind::TurnStarted { turn: first },
                user("第一轮用户请求"),
                thought(),
                tool("first-read"),
                agent("第一轮结论"),
                finished(first),
                EventKind::TurnStarted { turn: second },
                user("第二轮用户请求"),
                thought(),
                tool("second-read"),
                agent("**先检查**"),
                tool("second-verify"),
                agent(&answer),
            ];
            events.extend(match tail_kind {
                "tool" => vec![thought(), tool("trailing-tool")],
                "thought" => vec![tool("trailing-tool"), thought()],
                _ => vec![thought()],
            });
            let ids = deliver(&fixture, events, window, cx);
            let tail_id = ids.last().expect("last activity item").clone();
            let (row_id, user_row, grouped) = {
                let conversation = fixture
                    .view
                    .read(cx)
                    .conversation
                    .as_ref()
                    .expect("active conversation");
                let row = conversation.rows.last().expect("tail activity row");
                let grouped = row.items.len() > 1;
                let id = conversation.items[row.items.start].id();
                (
                    if grouped {
                        format!("activity-{id}")
                    } else {
                        id
                    },
                    conversation.row_for_item(4).expect("second user row"),
                    grouped,
                )
            };
            assert_ne!(user_row, 4, "activity groups change raw item indices");
            fixture.view.update(cx, |view, _| {
                view.composer_expanded = false;
                view.output_open = true;
                view.list.set_follow_mode(FollowMode::Tail);
            });
            window.resize(size(px(1120.), px(1000.)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            assert!(
                window
                    .try_find(format!("message-actions-{tail_id}"))
                    .is_none()
            );
            let working_height = window.find(row_id.clone()).bounds().size.height;
            deliver(&fixture, [finished(second)], window, cx);

            for compact in [false, true] {
                fixture.view.update(cx, |view, cx| {
                    view.compact = compact;
                    view.resize_window(window, cx);
                    view.list.set_follow_mode(FollowMode::Tail);
                });
                window.resize(size(px(1120.), px(1000.)));
                window.bounds_changed(cx);
                window.render_frame(cx);
                let row = window.find(row_id.clone()).bounds();
                let actions = window.find(format!("message-actions-{tail_id}"));
                assert!(actions.visible());
                assert!(
                    window
                        .try_find(format!("scroll-to-top-{tail_id}"))
                        .is_none()
                );
                assert_global_navigation_group(window, compact);
                let actions = actions.bounds();
                let disclosure = window
                    .find(if grouped {
                        format!("expand-{row_id}")
                    } else {
                        format!("expand-{tail_id}")
                    })
                    .bounds();
                assert!(row.size.height > working_height);
                assert!(actions.top() >= disclosure.bottom());
                assert!(actions.bottom() <= row.bottom());
                assert!(
                    window
                        .try_find(format!("message-actions-{}", ids[9]))
                        .is_none()
                );
                if grouped {
                    assert!(!fixture.view.read(cx).expanded.contains(&row_id));
                    assert!(window.try_find(tail_id.clone()).is_none());
                }
                window.hover(format!("message-actions-{tail_id}"), cx);
                window.click(format!("copy-response-{tail_id}"), cx);
                assert_eq!(
                    cx.read_from_clipboard().expect("copied response").text(),
                    Some(format!("**先检查**\n\n{answer}"))
                );
                window.click(format!("scroll-to-user-{tail_id}"), cx);
                let scroll = fixture.view.read(cx).list.logical_scroll_top();
                assert_eq!(scroll.item_ix, user_row);
                assert_eq!(scroll.offset_in_item, px(0.));
                window.render_frame(cx);
                assert_eq!(
                    fixture.view.read(cx).list.logical_scroll_top().item_ix,
                    user_row
                );
                fixture.view.read(cx).list.set_follow_mode(FollowMode::Tail);
                window.render_frame(cx);
                window.click("scroll-to-top", cx);
                assert_eq!(fixture.view.read(cx).list.logical_scroll_top().item_ix, 0);
            }

            fixture.view.update(cx, |view, cx| {
                view.list.set_follow_mode(FollowMode::Tail);
                view.update(
                    Update::Permissions {
                        epoch: view.epoch,
                        pending: vec![basr::model::PendingPermission {
                            request_id: "tail-permission".into(),
                            request: basr::acp::RequestPermissionRequest::new(
                                meta.id.to_string(),
                                basr::acp::ToolCallUpdate::new(
                                    "write",
                                    basr::acp::ToolCallUpdateFields::new(),
                                ),
                                Vec::new(),
                            ),
                        }],
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            assert!(
                window
                    .try_find(format!("message-actions-{tail_id}"))
                    .is_none()
            );
            fixture.view.update(cx, |view, cx| {
                view.update(
                    Update::Permissions {
                        epoch: view.epoch,
                        pending: Vec::new(),
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            assert!(window.find(format!("message-actions-{tail_id}")).visible());
        })
        .expect("tail activity actions in both window modes");
    }
}

#[gpui_kit::test]
fn startup_response_copies_in_initialization_and_navigates_safely_in_the_timeline(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let startup = fixture.view.update(cx, |view, cx| {
            view.composer_expanded = true;
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: meta.clone().into(),
                    events: vec![Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        agent("**pi v0.87.1**"),
                    )],
                },
                window,
                cx,
            );
            view.conversation.as_ref().expect("conversation").items[0].id()
        });
        window.render_frame(cx);
        assert!(window.find("agent-initialization-messages").visible());
        assert!(!window.find(format!("copy-response-{startup}")).visible());
        assert!(
            window
                .try_find(format!("scroll-to-user-{startup}"))
                .is_none()
        );
        assert!(
            window
                .try_find(format!("scroll-to-top-{startup}"))
                .is_none()
        );
        assert!(window.try_find("conversation-navigation").is_none());
        window.hover(format!("message-actions-{startup}"), cx);
        assert!(window.find(format!("copy-response-{startup}")).visible());
        window.click(format!("copy-response-{startup}"), cx);
        assert_eq!(
            cx.read_from_clipboard().expect("startup copy").text(),
            Some("**pi v0.87.1**".into())
        );

        for compact in [false, true] {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.composer_expanded = false;
                view.output_open = true;
                view.resize_window(window, cx);
            });
            window.render_frame(cx);
            assert!(
                window
                    .try_find(format!("scroll-to-top-{startup}"))
                    .is_none()
            );
            assert_global_navigation_group(window, compact);
            window.click("scroll-to-top", cx);
            window.click(format!("scroll-to-user-{startup}"), cx);
            assert!(fixture.view.read(cx).list.is_following_tail());
        }

        let turn = TurnId::new();
        let answer = format!("任务完成\n\n{}", "长回复的详细说明。\n\n".repeat(80));
        deliver(
            &fixture,
            [
                EventKind::TurnStarted { turn },
                user("开始任务"),
                agent(&answer),
                finished(turn),
            ],
            window,
            cx,
        );
        window.resize(size(px(1120.), px(1000.)));
        window.bounds_changed(cx);
        fixture
            .view
            .read(cx)
            .list
            .scroll_to(gpui_kit::ListOffset::default());
        window.render_frame(cx);
        window.click(format!("scroll-to-user-{startup}"), cx);
        let view = fixture.view.read(cx);
        let row = view
            .conversation
            .as_ref()
            .expect("conversation")
            .row_for_item(1)
            .expect("first user row");
        assert_eq!(
            view.list.logical_scroll_top().item_ix,
            row,
            "startup response without an associated user falls back to the recent user"
        );
    })
    .expect("startup copy and navigation fallback");
}
