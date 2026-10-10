use super::*;

#[gpui_kit::test]
fn tool_name_and_real_before_after_diff_are_visible(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let tool = basr::acp::ToolCall::new("edit", "修改配置")
                .name("apply_patch")
                .content(vec![
                    basr::acp::ToolCallContent::Diff(
                        basr::acp::Diff::new("/work/config.rs", "after\n").old_text("before\n"),
                    ),
                    basr::acp::ToolCallContent::Diff(basr::acp::Diff::new(
                        "/work/other.rs",
                        "known after only",
                    )),
                ]);
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCall(tool)),
                    ),
                },
                window,
                cx,
            );
            view.expanded.insert("tool-edit".into());
        });
        window.render_frame(cx);
        assert!(window.find("tool-name-tool-edit").bounds().size.height > px(0.));
        let diff = fixture.view.read(cx).markdown.borrow()["tool-edit-diff-0"]
            .state
            .clone();
        diff.update(cx, |state, cx| {
            state.select_all(cx);
            let text = state.selected_text();
            assert!(text.contains("-before\n+after"), "{text}");
            assert!(text.contains("@@ -1 +1 @@"), "{text}");
        });
        let after = fixture.view.read(cx).markdown.borrow()["tool-edit-diff-1"]
            .state
            .clone();
        after.update(cx, |state, cx| {
            state.select_all(cx);
            let text = state.selected_text();
            assert!(text.contains("修改后："), "{text}");
            assert!(text.contains("known after only"), "{text}");
            assert!(!text.contains("+++"), "{text}");
        });
    })
    .expect("expanded tool preview");
}

#[gpui_kit::test]
fn empty_terminal_exit_keeps_raw_result_and_displays_command_and_directory(
    cx: &mut TestAppContext,
) {
    for exit in [serde_json::json!({"exit_code": 0}), serde_json::json!({})] {
        let fixture = fixture(cx);
        let session = session();
        attach(&fixture, session.clone(), cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                let mut exit = exit.clone();
                exit["terminal_id"] = serde_json::json!("remote");
                let tool = basr::acp::ToolCall::new("shell", "Run shell command")
                    .content(vec![basr::acp::ToolCallContent::Terminal(
                        basr::acp::Terminal::new("remote"),
                    )])
                    .raw_output(serde_json::json!("raw result"))
                    .meta(serde_json::from_value::<basr::acp::Meta>(serde_json::json!({
                        "terminal_info": {
                            "terminal_id": "remote", "cwd": "/work/project", "command": "cargo test"
                        },
                        "terminal_exit": exit
                    })).expect("terminal metadata"));
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(
                            session.id,
                            Cursor::from_raw(1),
                            EventKind::Acp(basr::acp::SessionUpdate::ToolCall(tool)),
                        ),
                    },
                    window,
                    cx,
                );
                view.expanded.insert("tool-shell".into());
            });
            window.render_frame(cx);
            assert!(window.find("terminal-cwd-remote").bounds().size.height > px(0.));
            assert!(window.find("terminal-exit-remote").bounds().size.height > px(0.));
            let view = fixture.view.read(cx);
            let documents = view.markdown.borrow();
            assert!(documents.contains_key("tool-shell-terminal-command-remote"));
            assert!(documents.contains_key("tool-shell-output"));
            assert!(!documents.contains_key("tool-shell-terminal-remote"));
        })
        .expect("empty terminal state");
    }
}
