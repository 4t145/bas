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
        assert!(window.find("tool-output-tool-edit").visible());
        assert!(window.try_find("tool-input-tool-edit").is_none());
        window.click("expand-input-open-tool-edit", cx);
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
            assert!(window.find("terminal-exit-remote").bounds().size.height > px(0.));
            assert!(window.try_find("terminal-cwd-remote").is_none());
            window.click("expand-input-open-tool-shell", cx);
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

#[gpui_kit::test]
fn output_precedes_collapsed_input_and_sections_keep_their_state_during_stream_updates(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let command = "bash -lc 'echo first\n  echo second'";
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let tool = basr::acp::ToolCall::new("io", command)
                .status(basr::acp::ToolCallStatus::InProgress)
                .raw_input(serde_json::json!({"command": command, "verbose": true}))
                .raw_output(serde_json::json!("duplicate fallback"))
                .content(vec![basr::acp::ToolCallContent::Terminal(
                    basr::acp::Terminal::new("remote-io"),
                )])
                .meta(serde_json::from_value::<basr::acp::Meta>(serde_json::json!({
                    "terminal_info": {"terminal_id": "remote-io", "cwd": "/work", "command": command},
                    "terminal_output": {"terminal_id": "remote-io", "data": "native output"}
                })).expect("terminal metadata"));
            view.update(Update::Event {
                epoch: view.epoch,
                event: Event::new(session.id, Cursor::from_raw(1), EventKind::Acp(
                    basr::acp::SessionUpdate::ToolCall(tool),
                )),
            }, window, cx);
            view.expanded.insert("tool-io".into());
        });
        window.render_frame(cx);
        assert!(window.find("tool-output-tool-io").visible());
        assert!(window.try_find("tool-input-tool-io").is_none());
        assert!(window.find("tool-output-section-tool-io").bounds().top()
            < window.find("tool-input-section-tool-io").bounds().top());
        let output = fixture.view.read(cx).markdown.borrow()["tool-io-terminal-remote-io"]
            .state.clone();
        let entity = output.entity_id();
        assert!(!fixture.view.read(cx).markdown.borrow().contains_key("tool-io-output"),
            "native terminal output suppresses the duplicate raw fallback");

        window.click("expand-input-open-tool-io", cx);
        window.render_frame(cx);
        assert!(window.find("tool-input-tool-io").visible());
        assert!(window.find("tool-output-tool-io").visible());
        let documents = fixture.view.read(cx).markdown.borrow();
        assert!(documents.contains_key("tool-io-input"));
        let input_command = documents["tool-io-input-command"].state.clone();
        let parameters = documents["tool-io-input"].state.clone();
        assert!(!documents.contains_key("tool-io-command"));
        assert!(!documents.contains_key("tool-io-terminal-command-remote-io"),
            "the command is already in the JSON input");
        drop(documents);
        input_command.update(cx, |state, cx| {
            state.select_all(cx);
            assert_eq!(state.selected_text().trim(), command,
                "the shell command is shown once with real newlines");
        });
        parameters.update(cx, |state, cx| {
            state.select_all(cx);
            let text = state.selected_text();
            let parameters: serde_json::Value = serde_json::from_str(text.trim())
                .expect("remaining parameters are JSON");
            assert_eq!(parameters, serde_json::json!({"verbose": true}));
        });
        assert_eq!(fixture.view.read(cx).conversation.as_ref().expect("conversation")
            .items.iter().find_map(|item| match item {
                Item::Tool(tool) => tool.raw_input.clone(),
                _ => None,
            }), Some(serde_json::json!({"command": command, "verbose": true})),
            "display formatting does not alter the ACP input");
        window.click("expand-output-closed-tool-io", cx);
        window.render_frame(cx);
        assert!(window.try_find("tool-output-tool-io").is_none());
        assert!(window.find("tool-input-tool-io").visible());

        fixture.view.update(cx, |view, cx| {
            view.update(Update::Event {
                epoch: view.epoch,
                event: Event::new(session.id, Cursor::from_raw(2), EventKind::Acp(
                    basr::acp::SessionUpdate::ToolCallUpdate(basr::acp::ToolCallUpdate::new(
                        "io", basr::acp::ToolCallUpdateFields::new()
                            .status(basr::acp::ToolCallStatus::Completed),
                    ).meta(serde_json::from_value::<basr::acp::Meta>(serde_json::json!({
                        "terminal_output": {"terminal_id": "remote-io", "data": "\nsecond result"}
                    })).expect("stream metadata"))),
                )),
            }, window, cx);
            assert!(view.expanded.contains("input-open-tool-io"));
            assert!(view.expanded.contains("output-closed-tool-io"));
        });
        window.render_frame(cx);
        assert!(window.try_find("tool-output-tool-io").is_none());
        assert!(window.find("tool-input-tool-io").visible());
        window.click("expand-input-open-tool-io", cx);
        window.render_frame(cx);
        assert!(window.try_find("tool-input-tool-io").is_none());
        assert!(window.try_find("tool-output-tool-io").is_none());
        window.click("expand-output-closed-tool-io", cx);
        window.render_frame(cx);
        assert!(window.find("tool-output-tool-io").visible());
        assert!(window.try_find("tool-input-tool-io").is_none());
        assert_eq!(fixture.view.read(cx).markdown.borrow()["tool-io-terminal-remote-io"]
            .state.entity_id(), entity);
        output.update(cx, |state, cx| {
            state.select_all(cx);
            let text = state.selected_text();
            assert!(text.contains("native output"), "{text}");
            assert!(text.contains("second result"), "{text}");
        });
    }).expect("independent tool sections");
}

#[gpui_kit::test]
fn unrelated_raw_input_keeps_the_full_multiline_command_in_the_input_section(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let session = session();
    attach(&fixture, session.clone(), cx);
    let command = "bash -lc 'echo first\n  printf \"second\\n\"'";
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        session.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::ToolCall(
                            basr::acp::ToolCall::new("unrelated-input", command)
                                .raw_input(serde_json::json!({"cwd": "/work"})),
                        )),
                    ),
                },
                window,
                cx,
            );
            view.expanded.insert("tool-unrelated-input".into());
        });
        window.render_frame(cx);
        assert!(window.try_find("tool-input-tool-unrelated-input").is_none());
        window.click("expand-input-open-tool-unrelated-input", cx);
        window.render_frame(cx);
        let state = {
            let view = fixture.view.read(cx);
            let documents = view.markdown.borrow();
            assert!(documents.contains_key("tool-unrelated-input-input"));
            documents["tool-unrelated-input-command"].state.clone()
        };
        state.update(cx, |state, cx| {
            state.select_all(cx);
            assert_eq!(state.selected_text().trim(), command);
        });
    })
    .expect("full command and unrelated input");
}

#[gpui_kit::test]
fn tool_header_leaves_vertical_room_for_descenders_at_different_font_sizes(
    cx: &mut TestAppContext,
) {
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
                            basr::acp::ToolCall::new(
                                "descenders",
                                "running debugging gyp jq python 中文 ".repeat(8),
                            ),
                        )),
                    ),
                },
                window,
                cx,
            );
        });
        for (compact, font_size) in [(false, 14.), (true, 14.), (false, 24.), (true, 24.)] {
            let editor = theme::Editor::current(cx);
            let mut document = editor.load().expect("appearance document");
            let mut preferences = document.preferences.clone();
            preferences.content_font_size = Some(font_size);
            editor
                .save(&mut document, preferences, window, cx)
                .expect("apply font size");
            fixture.view.update(cx, |view, cx| {
                view.compact = compact;
                view.output_open = true;
                view.resize_window(window, cx);
                view.list.remeasure_items(0..1);
            });
            window.bounds_changed(cx);
            window.render_frame(cx);
            let button = window.find("expand-tool-descenders").bounds();
            let label = window.find("disclosure-label-tool-descenders").bounds();
            let status = window.find("status-tool-descenders").bounds();
            assert!(
                label.size.height >= px((font_size * 1.4).ceil()),
                "{label:?}"
            );
            assert!(
                label.top() >= button.top() + px(3.),
                "{label:?} within {button:?}"
            );
            assert!(
                label.bottom() <= button.bottom() - px(3.),
                "{label:?} within {button:?}"
            );
            assert!(label.right() <= status.left());
        }
    })
    .expect("tool header geometry");
}
