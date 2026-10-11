use super::*;

#[gpui_kit::test]
fn startup_agent_message_stays_formatted_and_separate_from_the_first_turn(cx: &mut TestAppContext) {
    use basr::acp::{ContentChunk, SessionUpdate};
    use basr::model::{Role, TurnId};

    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let mut startup_id = String::new();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: session.clone(),
                    events: vec![Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                            "**pi v0.87.1**\n\nAgent 已准备好。".into(),
                        ))),
                    )],
                },
                window,
                cx,
            );
            startup_id = view.conversation.as_ref().expect("conversation").items[0].id();
        });
        window.render_frame(cx);
        assert!(
            window
                .try_find(format!("message-source-{startup_id}"))
                .is_none()
        );
        let document = fixture.view.read(cx).markdown.borrow()[&format!("{startup_id}-0")]
            .state
            .clone();
        document.update(cx, |state, cx| {
            assert!(
                state.bounds().size.height > px(0.),
                "startup Markdown has content height on its first frame"
            );
            state.select_all(cx);
            assert_eq!(state.selected_text().trim(), "pi v0.87.1\nAgent 已准备好。");
        });
    })
    .expect("startup snapshot");
    cx.run_until_parked();

    let mut ids = Vec::new();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let turn = TurnId::new();
            let events = [
                EventKind::TurnStarted { turn },
                EventKind::Acp(SessionUpdate::UserMessageChunk(ContentChunk::new(
                    "**第一条消息**，请检查代码并保留初始化信息。".repeat(4).into(),
                ))),
                EventKind::Acp(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    "**开始检查**\n\n- 保留初始化消息\n- 单独显示用户请求".into(),
                ))),
                EventKind::TurnFinished {
                    turn,
                    result: Ok(basr::acp::PromptResponse::new(basr::model::StopReason::EndTurn)),
                },
            ];
            for (index, kind) in events.into_iter().enumerate() {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(session.id, Cursor::from_raw(index as u64 + 2), kind),
                    },
                    window,
                    cx,
                );
            }
            let conversation = view.conversation.as_ref().expect("conversation");
            assert_eq!(conversation.items.len(), 3);
            assert_eq!(conversation.rows.len(), 3);
            ids = conversation.items.iter().map(Item::id).collect();
            assert_eq!(ids[0], startup_id);
            for (item, role) in conversation.items.iter().zip([Role::Agent, Role::User, Role::Agent]) {
                assert!(matches!(item, Item::Message { message, streaming: false } if message.role == role));
            }
            assert_eq!(conversation.completed_reply.expect("first reply").index, 2);
            view.composer_expanded = false;
        });
        window.render_frame(cx);
        for id in &ids {
            assert!(window.try_find(format!("message-source-{id}")).is_none());
        }
        let startup = window.find(ids[0].clone()).bounds();
        let user = window.find(ids[1].clone()).bounds();
        let reply = window.find(ids[2].clone()).bounds();
        assert!(startup.bottom() <= user.top());
        assert!(user.bottom() <= reply.top());
        let document = fixture.view.read(cx).markdown.borrow()[&format!("{}-0", ids[1])]
            .state
            .clone();
        document.update(cx, |state, cx| {
            assert!(state.bounds().size.height > px(0.), "first user message has content height on its first frame");
            state.select_all(cx);
            assert_eq!(state.selected_text().trim(), "第一条消息，请检查代码并保留初始化信息。".repeat(4));
        });
    })
    .expect("first turn before Markdown parsing finishes");
    cx.run_until_parked();

    cx.update_window(fixture.window.into(), |_, window, cx| {
        for compact in [false, true] {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.output_open = true;
                view.resize_window(window, cx);
            });
            for width in [1120., 460.] {
                window.resize(size(px(width), px(800.)));
                window.bounds_changed(cx);
                window.render_frame(cx);
                let startup = window.find(ids[0].clone()).bounds();
                let user = window.find(ids[1].clone()).bounds();
                let reply = window.find(ids[2].clone()).bounds();
                assert!(startup.size.height > px(0.));
                assert!(user.size.height > px(0.));
                assert!(reply.size.height > px(0.));
                assert!(startup.bottom() <= user.top(), "startup and first user message must not overlap: startup={startup:?}, user={user:?}");
                assert!(user.bottom() <= reply.top(), "first user message and reply must not overlap: user={user:?}, reply={reply:?}");
                for (id, row) in ids.iter().zip([startup, user, reply]) {
                    let body = fixture.view.read(cx).markdown.borrow()[&format!("{id}-0")].state.read(cx).bounds();
                    assert!(body.top() >= row.top());
                    assert!(body.bottom() <= row.bottom(), "Markdown must stay within its message row: body={body:?}, row={row:?}");
                }
            }
        }
        let document = fixture.view.read(cx).markdown.borrow()[&format!("{startup_id}-0")].state.clone();
        document.update(cx, |state, cx| {
            state.select_all(cx);
            assert_eq!(state.selected_text().trim(), "pi v0.87.1\nAgent 已准备好。");
        });
    })
    .expect("formatted startup output across wrapping and compact layouts");
}

#[gpui_kit::test]
fn activity_groups_fold_together_and_late_updates_keep_the_answer_visible(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let mut activity = String::new();
    let mut answer = String::new();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let updates = [
                basr::acp::SessionUpdate::UserMessageChunk(basr::acp::ContentChunk::new(
                    "读取并检查".into(),
                )),
                basr::acp::SessionUpdate::AgentThoughtChunk(basr::acp::ContentChunk::new(
                    "检查配置和源码".into(),
                )),
                basr::acp::SessionUpdate::ToolCall(
                    basr::acp::ToolCall::new("read", "读取配置")
                        .status(basr::acp::ToolCallStatus::Completed),
                ),
                basr::acp::SessionUpdate::ToolCall(
                    basr::acp::ToolCall::new("search", "搜索源码")
                        .status(basr::acp::ToolCallStatus::InProgress),
                ),
                basr::acp::SessionUpdate::AgentMessageChunk(basr::acp::ContentChunk::new(
                    "已经找到配置问题".into(),
                )),
            ];
            for (index, update) in updates.into_iter().enumerate() {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(
                            session.id,
                            Cursor::from_raw(index as u64 + 1),
                            EventKind::Acp(update),
                        ),
                    },
                    window,
                    cx,
                );
            }
            let conversation = view.conversation.as_ref().expect("conversation");
            assert_eq!(conversation.rows.len(), 3);
            assert_eq!(view.list.item_count(), 3);
            activity = format!("activity-{}", conversation.items[1].id());
            answer = conversation.items[4].id();
        });
        window.render_frame(cx);
    })
    .expect("window");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        for compact in [false, true] {
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.output_open = true;
                view.resize_window(window, cx);
            });
            window.bounds_changed(cx);
            window.render_frame(cx);
            let toggle = format!("expand-{activity}");
            assert_eq!(
                window.find(toggle.clone()).label(),
                Some("1 次思考 · 2 次工具调用 · 1 个执行中"),
            );
            assert!(window.try_find("tool-read").is_none());
            assert!(window.find(answer.clone()).visible());
            let collapsed_height = window.find(activity.clone()).bounds().size.height;
            window.click(toggle.clone(), cx);
            assert!(window.find("tool-read").visible());
            assert!(window.find("tool-search").visible());
            assert!(window.find(activity.clone()).bounds().size.height > collapsed_height);
            window.click(toggle, cx);
            assert!(window.try_find("tool-read").is_none());
            assert!(window.find(answer.clone()).visible());
        }
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(6),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCallUpdate(
                            basr::acp::ToolCallUpdate::new(
                                "search",
                                basr::acp::ToolCallUpdateFields::new()
                                    .status(basr::acp::ToolCallStatus::Failed),
                            ),
                        )),
                    ),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(
            window.find(format!("expand-{activity}")).label(),
            Some("1 次思考 · 2 次工具调用 · 1 个失败"),
        );
        assert!(window.find(answer).visible());
        assert_eq!(fixture.view.read(cx).list.item_count(), 3);
    })
    .expect("window");
}

#[gpui_kit::test]
fn appending_activity_keeps_a_tool_the_user_opened_expanded(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCall(
                            basr::acp::ToolCall::new("first", "读取")
                                .raw_input(serde_json::json!({"path": "Cargo.toml"})),
                        )),
                    ),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        window.click("expand-tool-first", cx);
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(2),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCall(
                            basr::acp::ToolCall::new("second", "搜索"),
                        )),
                    ),
                },
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert!(window.find("tool-first").visible());
        assert!(window.find("tool-second").visible());
        assert!(
            fixture
                .view
                .read(cx)
                .expanded
                .contains("activity-tool-first")
        );
        assert!(fixture.view.read(cx).expanded.contains("tool-first"));
        window.click("expand-activity-tool-first", cx);
        assert!(window.try_find("tool-first").is_none());
        window.click("expand-activity-tool-first", cx);
        assert!(fixture.view.read(cx).expanded.contains("tool-first"));
    })
    .expect("window");
}

#[gpui_kit::test]
fn content_footers_return_to_nested_headers_and_preserve_sibling_sections(cx: &mut TestAppContext) {
    use basr::acp::{ContentChunk, SessionUpdate, ToolCall, ToolCallStatus};
    use basr::model::{StopReason, TurnId};

    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let thought_text = "分析每一项配置并记录处理结果。\n\n".repeat(80);
    let output = "构建输出：检查当前配置并继续处理下一项。\n".repeat(80);
    let mut thought_id = String::new();
    let mut activity_id = String::new();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let turn = TurnId::new();
            let events = [
                EventKind::TurnStarted { turn },
                EventKind::Acp(SessionUpdate::UserMessageChunk(ContentChunk::new(
                    "检查配置".into(),
                ))),
                EventKind::Acp(SessionUpdate::AgentThoughtChunk(ContentChunk::new(
                    thought_text.clone().into(),
                ))),
                EventKind::Acp(SessionUpdate::ToolCall(
                    ToolCall::new("nested", "运行构建")
                        .status(ToolCallStatus::Completed)
                        .raw_input(serde_json::json!({"command": "cargo build"}))
                        .raw_output(serde_json::Value::String(output.clone())),
                )),
                EventKind::Acp(SessionUpdate::ToolCall(
                    ToolCall::new("sibling", "检查输出")
                        .status(ToolCallStatus::Completed)
                        .raw_input(serde_json::json!({"path": "build.log"}))
                        .raw_output(serde_json::Value::String(output.clone())),
                )),
                EventKind::Acp(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    "检查完成，下面是完整结果。\n\n".repeat(100).into(),
                ))),
                EventKind::TurnFinished {
                    turn,
                    result: Ok(basr::acp::PromptResponse::new(StopReason::EndTurn)),
                },
            ];
            for (index, kind) in events.into_iter().enumerate() {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(session.id, Cursor::from_raw(index as u64 + 1), kind),
                    },
                    window,
                    cx,
                );
            }
            thought_id = view.conversation.as_ref().expect("conversation").items[1].id();
            activity_id = format!("activity-{thought_id}");
            view.expanded.extend([
                thought_id.clone(),
                activity_id.clone(),
                "tool-nested".into(),
                "tool-sibling".into(),
                "input-open-tool-nested".into(),
                "output-closed-tool-sibling".into(),
            ]);
            view.composer_expanded = false;
            view.list.set_follow_mode(FollowMode::Normal);
            view.list.scroll_to(gpui_kit::ListOffset::default());
        });
        window.render_frame(cx);
    })
    .expect("expanded grouped content");
    cx.run_until_parked();

    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let nested_offset = fixture
            .view
            .read(cx)
            .content_scroll_offset("tool-nested", 1)
            .offset_in_item;
        assert!(
            nested_offset > px(800.),
            "the nested tool follows a long thought"
        );

        for id in ["tool-nested", thought_id.as_str()] {
            let expected = fixture.view.read(cx).content_scroll_offset(id, 1);
            scroll_content_footer_into_view(&fixture, id, window, cx);
            window.click(format!("scroll-to-content-{id}"), cx);
            let actual = fixture.view.read(cx).list.logical_scroll_top();
            assert_eq!(actual.item_ix, expected.item_ix);
            assert!(
                (actual.offset_in_item - expected.offset_in_item).abs() < px(1.),
                "return {id}: actual={actual:?}, expected={expected:?}",
            );
            assert!(window.find(format!("expand-{id}")).visible());

            scroll_content_footer_into_view(&fixture, id, window, cx);
            window.click(format!("collapse-content-{id}"), cx);
            let view = fixture.view.read(cx);
            assert!(!view.expanded.contains(id));
            assert!(view.expanded.contains(&activity_id));
            assert!(view.expanded.contains("tool-sibling"));
            assert!(view.expanded.contains("input-open-tool-nested"));
            assert!(view.expanded.contains("output-closed-tool-sibling"));
            let actual = view.list.logical_scroll_top();
            assert!(
                actual.item_ix < expected.item_ix
                    || (actual.item_ix == expected.item_ix
                        && actual.offset_in_item <= expected.offset_in_item),
                "collapse may clamp at the end of the shorter row: actual={actual:?}, expected={expected:?}",
            );
            let header = window.find(format!("expand-{id}"));
            assert!(header.visible());
            let viewport = view.list.viewport_bounds();
            assert!(header.bounds().top() >= viewport.top());
            assert!(header.bounds().bottom() <= viewport.bottom());
        }

        scroll_content_footer_into_view(&fixture, &activity_id, window, cx);
        window.click(format!("collapse-content-{activity_id}"), cx);
        assert!(!fixture.view.read(cx).expanded.contains(&activity_id));
        assert!(fixture.view.read(cx).expanded.contains("tool-sibling"));
        assert!(window.try_find("tool-sibling").is_none());
        assert!(window.find(format!("expand-{activity_id}")).visible());
        window.click(format!("expand-{activity_id}"), cx);
        assert!(window.find("tool-sibling").visible());
        assert!(
            fixture
                .view
                .read(cx)
                .expanded
                .contains("output-closed-tool-sibling")
        );
    })
    .expect("footer navigation and selective collapse");
}

fn scroll_content_footer_into_view(fixture: &Fixture, id: &str, window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    let view = fixture.view.read(cx);
    let footer = window.find(format!("content-actions-{id}")).bounds();
    let row = view.list.bounds_for_item(1).expect("painted activity row");
    view.list.scroll_to(gpui_kit::ListOffset {
        item_ix: 1,
        offset_in_item: (footer.top() - row.top() - view.list.viewport_bounds().size.height / 2.)
            .max(px(0.)),
    });
    window.render_frame(cx);
}

#[gpui_kit::test]
fn thought_footer_copy_only_appears_on_hover_and_keeps_the_thought_open(cx: &mut TestAppContext) {
    use basr::acp::{ContentChunk, SessionUpdate};
    use basr::model::{StopReason, TurnId};

    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let thought_id = fixture.view.update(cx, |view, cx| {
            let turn = TurnId::new();
            for (index, kind) in [
                EventKind::TurnStarted { turn },
                EventKind::Acp(SessionUpdate::AgentThoughtChunk(ContentChunk::new(
                    "**先检查**\n\n保留原始思考内容。".into(),
                ))),
                EventKind::TurnFinished {
                    turn,
                    result: Ok(basr::acp::PromptResponse::new(StopReason::EndTurn)),
                },
            ]
            .into_iter()
            .enumerate()
            {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(session.id, Cursor::from_raw(index as u64 + 1), kind),
                    },
                    window,
                    cx,
                );
            }
            view.composer_expanded = false;
            view.conversation.as_ref().expect("conversation").items[0].id()
        });
        window.render_frame(cx);
        window.click(format!("expand-{thought_id}"), cx);
        assert!(!window.find(format!("copy-{thought_id}")).visible());
        assert!(
            window
                .find(format!("scroll-to-content-{thought_id}"))
                .visible()
        );
        assert!(
            window
                .find(format!("collapse-content-{thought_id}"))
                .visible()
        );
        window.hover(format!("content-actions-{thought_id}"), cx);
        assert!(window.find(format!("copy-{thought_id}")).visible());
        window.click(format!("copy-{thought_id}"), cx);
        assert_eq!(
            cx.read_from_clipboard().expect("copied thought").text(),
            Some("**先检查**\n\n保留原始思考内容。".into()),
        );
        assert!(fixture.view.read(cx).expanded.contains(&thought_id));
    })
    .expect("thought footer copy");
}
