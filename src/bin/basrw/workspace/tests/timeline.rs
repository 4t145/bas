use super::*;

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
