use super::*;

fn save_enter_behavior(mode: composer::Mode, window: &mut Window, cx: &mut App) {
    let editor = theme::Editor::current(cx);
    let mut document = editor.load().expect("appearance settings");
    let mut preferences = document.preferences.clone();
    preferences.composer_enter_behavior = mode;
    editor
        .save(&mut document, preferences, window, cx)
        .expect("save Enter preference");
}

#[gpui_kit::test]
fn new_thread_editor_fills_the_content_area_until_the_first_prompt(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(fixture.view.read(cx).composer_fills_thread());
        assert!(window.try_find("conversation-timeline").is_none());
    })
    .expect("initial new thread fills the window");
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.press("ctrl-n", cx);
        window.input("new thread draft", cx);
        for (width, height) in [(600., 400.), (1500., 1400.), (600., 400.)] {
            window.resize(size(px(width), px(height)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            window.render_frame(cx);
            let body = window.find("thread-body").bounds();
            let panel = window.find("composer-panel").bounds();
            let editor = window.find("composer-editor").bounds();
            let toolbar = window.find("composer-toolbar").bounds();
            let expand = window.find("expand-message-editor").bounds();
            let send = window.find("send-message").bounds();
            let margin = panel.left() - body.left();
            assert_eq!(margin, px(chat::THREAD_BODY_PADDING));
            assert_eq!(body.right() - panel.right(), margin);
            assert_eq!(body.bottom() - panel.bottom(), px(chat::COMPOSER_SIDE_PADDING));
            assert!(panel.size.height >= body.size.height - px(60.));
            assert!(editor.size.height >= body.size.height - px(110.));
            assert_eq!(editor.bottom(), toolbar.top());
            assert!(toolbar.bottom() <= panel.bottom());
            assert_eq!(expand.center().x, send.center().x);
            assert!(expand.top() >= editor.top() && expand.bottom() <= editor.bottom());
            assert!(window.try_find("composer-header").is_none());
            assert!(window.try_find("conversation-timeline").is_none());
            assert_eq!(
                window.find("sidebar-header").bounds().bottom(),
                window.find("thread-header").bounds().bottom(),
            );
        }
        window.click("expand-message-editor", cx);
        window.render_frame(cx);
        assert!(window.find("conversation-timeline").visible());
        assert_eq!(fixture.view.read(cx).composer.read(cx).value(), "new thread draft");
        window.click("expand-message-editor", cx);
        let meta = session();
        fixture.view.update(cx, |view, cx| {
            view.select_agent("embedded".into(), window, cx);
            view.update(
                Update::Created { epoch: view.epoch, result: Ok(meta.clone()) },
                window,
                cx,
            );
            view.update(
                Update::Snapshot { epoch: view.epoch, session: meta, events: Vec::new() },
                window,
                cx,
            );
            view.stream = StreamState::Live;
        });
        window.render_frame(cx);
        assert!(fixture.view.read(cx).composer_fills_thread());
        assert!(fixture.view.read(cx).can_select_agent());
        while fixture.commands.try_recv().is_ok() {}
        window.press("ctrl-enter", cx);
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).composer_expanded);
        assert!(window.find("conversation-timeline").visible());
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "new thread draft"));
    })
    .expect("new thread editor resize, collapse, initialization and first send");
}

#[gpui_kit::test]
fn startup_information_keeps_the_new_thread_editor_expanded(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let mut meta = session();
    meta.agent = "pi".into();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(meta.clone()),
                },
                window,
                cx,
            );
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: meta.clone(),
                    events: vec![Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::AgentMessageChunk(
                            basr::acp::ContentChunk::new("**pi v0.87.1**".into()),
                        )),
                    )],
                },
                window,
                cx,
            );
            view.stream = StreamState::Live;
        });
        window.render_frame(cx);
        assert!(fixture.view.read(cx).composer_fills_thread());
        assert!(fixture.view.read(cx).can_select_agent());
        assert!(window.try_find("conversation-timeline").is_none());
        let information = window.find("agent-initialization-messages").bounds();
        let editor = window.find("composer-editor").bounds();
        assert!(information.size.height > px(0.) && information.size.height <= px(160.));
        assert!(information.bottom() <= editor.top());
        assert!(editor.size.height > window.viewport_size().height / 2.);
        while fixture.commands.try_recv().is_ok() {}
        window.input("检查项目", cx);
        window.press("ctrl-enter", cx);
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).composer_fills_thread());
        assert!(!fixture.view.read(cx).can_select_agent());
        assert!(window.try_find("agent-initialization-messages").is_none());
        assert!(window.find("conversation-timeline").visible());
        assert!(matches!(fixture.commands.try_recv(), Ok(Command::Prompt { text, .. }) if text == "检查项目"));
    })
    .expect("startup information and first prompt use separate layout regions");
}

#[gpui_kit::test]
fn expanding_an_initialized_unused_compact_thread_fills_the_editor_until_first_send(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let meta = session();
    let directory = tempfile::tempdir().expect("draft attachment directory");
    let path = directory.path().join("draft.txt");
    std::fs::write(&path, "keep this attachment").expect("draft attachment");
    let attachment = Attachment::load(&path).expect("load draft attachment");
    let draft = format!("check the project {}", attachment.mention);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.compact = true;
            view.composer_expanded = false;
            view.sync_composer_mode(cx);
            view.resize_window(window, cx);
            view.attachments = vec![attachment.clone()];
            view.composer.update(cx, |input, cx| {
                input.set_value(draft.clone(), window, cx);
                input.set_selected_range(1..5, cx);
                input.focus(window, cx);
            });
            view.update(
                Update::Created {
                    epoch: fixture.initial_create.epoch,
                    result: Ok(meta.clone()),
                },
                window,
                cx,
            );
            view.update(
                Update::Snapshot {
                    epoch: view.epoch,
                    session: meta.clone(),
                    events: vec![Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::AgentMessageChunk(
                            basr::acp::ContentChunk::new("**pi v0.87.1**".into()),
                        )),
                    )],
                },
                window,
                cx,
            );
            view.stream = StreamState::Live;
            assert_eq!(view.active, Some(meta.id));
            assert!(!view.composer_expanded);
            assert!(!view.thread_has_user_prompt());
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.click("toggle-mode", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert!(!view.compact);
        assert!(view.composer_expanded);
        assert!(view.composer_fills_thread());
        assert_eq!(view.active, Some(meta.id));
        assert_eq!(view.composer.read(cx).value(), draft);
        assert_eq!(view.composer.read(cx).selected_range(), 1..5);
        assert_eq!(view.attachments[0].id, attachment.id);
        assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
        assert!(window.try_find("conversation-timeline").is_none());
        let information = window.find("agent-initialization-messages").bounds();
        let editor = window.find("composer-editor").bounds();
        let body = window.find("thread-body").bounds();
        assert!(information.size.height > px(0.));
        assert!(information.bottom() <= editor.top());
        assert!(editor.size.height > body.size.height / 2.);
        while fixture.commands.try_recv().is_ok() {}
        window.click("send-message", cx);
        assert!(
            matches!(fixture.commands.try_recv(), Ok(Command::Prompt { session, text, attachments, .. })
                if session == meta.id && text == draft && attachments[0].id == attachment.id)
        );
        fixture.view.update(cx, |view, cx| {
            let accepted = receipt(2);
            view.update(
                Update::Sent {
                    session: meta.id,
                    text: draft.clone(),
                    attachments: vec![attachment.clone()],
                    result: Ok(accepted),
                },
                window,
                cx,
            );
            for (cursor, kind) in [
                (2, EventKind::TurnStarted { turn: accepted.turn }),
                (
                    3,
                    EventKind::Acp(basr::acp::SessionUpdate::UserMessageChunk(
                        basr::acp::ContentChunk::new(draft.clone().into()),
                    )),
                ),
                (
                    4,
                    EventKind::TurnFinished {
                        turn: accepted.turn,
                        result: Ok(basr::acp::PromptResponse::new(basr::acp::StopReason::EndTurn)),
                    },
                ),
            ] {
                view.update(
                    Update::Event {
                        epoch: view.epoch,
                        event: Event::new(meta.id, Cursor::from_raw(cursor), kind),
                    },
                    window,
                    cx,
                );
            }
            view.composer.update(cx, |input, cx| {
                input.set_value("follow-up draft", window, cx);
            });
            assert!(view.thread_has_user_prompt());
            assert!(!view.composer_expanded);
        });
        window.render_frame(cx);
        window.click("toggle-mode", cx);
        window.render_frame(cx);
        assert!(fixture.view.read(cx).compact);
        window.click("toggle-mode", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert!(!view.compact);
        assert!(!view.composer_expanded);
        assert!(!view.composer_fills_thread());
        assert_eq!(view.active, Some(meta.id));
        assert_eq!(view.composer.read(cx).value(), "follow-up draft");
        assert!(window.try_find("agent-initialization-messages").is_none());
        assert!(window.find("conversation-timeline").visible());
    })
    .expect("window mode expands an initialized draft while preserving established conversations");
}

#[gpui_kit::test]
fn leading_bang_stays_ordinary_agent_text_in_the_collapsed_composer(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.input("! printf literal", cx);
    })
    .expect("ordinary punctuation in the composer");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert!(!view.composer_expanded);
        assert_eq!(view.composer_mode, composer::Mode::Send);
        assert_eq!(view.composer.read(cx).value(), "! printf literal");
        window.press("enter", cx);
    })
    .expect("ordinary Enter behavior remains enabled");
    cx.run_until_parked();
    let Command::Prompt { session, text, .. } =
        fixture.commands.try_recv().expect("ordinary agent prompt")
    else {
        panic!("expected Prompt");
    };
    assert_eq!(session, meta.id);
    assert_eq!(text, "! printf literal");
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn composer_expansion_button_and_shortcut_preserve_draft_selection_and_focus(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("保留草稿😀\nsecond line", window, cx);
                input.set_selected_range(0.."保留草稿😀".len(), cx);
                input.focus(window, cx);
            });
        });
        window.render_frame(cx);
        window.render_frame(cx);
        let collapsed_height = window.find("composer-editor").bounds().size.height;
        for use_button in [true, false] {
            if use_button {
                window.click("expand-message-editor", cx);
            } else {
                window.press("alt-shift-escape", cx);
            }
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert!(view.composer_expanded);
            assert_eq!(view.composer_mode, composer::Mode::Newline);
            assert_eq!(view.composer.read(cx).value(), "保留草稿😀\nsecond line");
            assert_eq!(
                view.composer.read(cx).selected_range(),
                0.."保留草稿😀".len()
            );
            assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
            assert!(window.find("composer-editor").bounds().size.height > collapsed_height);
            if use_button {
                window.click("expand-message-editor", cx);
            } else {
                window.press("alt-shift-escape", cx);
            }
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert!(!view.composer_expanded);
            assert_eq!(view.composer_mode, composer::Mode::Send);
            assert_eq!(view.composer.read(cx).value(), "保留草稿😀\nsecond line");
            assert_eq!(
                view.composer.read(cx).selected_range(),
                0.."保留草稿😀".len()
            );
            assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(
                window.find("composer-editor").bounds().size.height,
                collapsed_height
            );
        }
    })
    .expect("expand and collapse the same editor");
}

#[gpui_kit::test]
fn expanded_composer_enter_inserts_a_newline_and_control_enter_sends(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.input("first line", cx);
        window.press("alt-shift-escape", cx);
        window.press("enter", cx);
        window.input("second line", cx);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "first line\nsecond line"
        );
        assert!(fixture.commands.try_recv().is_err());
        window.press("ctrl-enter", cx);
        let Command::Prompt { text, .. } = fixture.commands.try_recv().expect("explicit send")
        else {
            panic!("expected prompt");
        };
        assert_eq!(text, "first line\nsecond line");
        assert!(fixture.commands.try_recv().is_err());
    })
    .expect("expanded keyboard behavior");
}

#[gpui_kit::test]
fn collapsed_composer_restores_enter_sending(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.input("send after collapse", cx);
        window.press("alt-shift-escape", cx);
        window.press("alt-shift-escape", cx);
        window.press("enter", cx);
    })
    .expect("collapsed keyboard behavior");
    cx.run_until_parked();
    let Command::Prompt { text, .. } = fixture.commands.try_recv().expect("Enter sends again")
    else {
        panic!("expected prompt");
    };
    assert_eq!(text, "send after collapse");
    assert!(fixture.commands.try_recv().is_err());
}

#[gpui_kit::test]
fn expanded_composer_defers_new_enter_preferences_until_collapsed(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    for mode in [composer::Mode::Newline, composer::Mode::Send] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("draft\nmore", window, cx);
                    input.set_selected_range(1..4, cx);
                });
                view.toggle_composer_expansion(window, cx);
                view.begin_rename(window, cx);
            });
            save_enter_behavior(mode, window, cx);
        })
        .expect("change preference while expanded");
        cx.run_until_parked();
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert!(view.composer_expanded);
            assert_eq!(view.composer_mode, composer::Mode::Newline);
            assert_eq!(view.composer.read(cx).value(), "draft\nmore");
            assert_eq!(view.composer.read(cx).selected_range(), 1..4);
            assert!(view.rename.read(cx).focus_handle(cx).is_focused(window));
            fixture
                .view
                .update(cx, |view, cx| view.toggle_composer_expansion(window, cx));
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert!(!view.composer_expanded);
            assert_eq!(view.composer_mode, mode);
            assert_eq!(view.composer.read(cx).value(), "draft\nmore");
            assert_eq!(view.composer.read(cx).selected_range(), 1..4);
            assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
            window.press("enter", cx);
            if mode == composer::Mode::Newline {
                assert_eq!(
                    fixture.view.read(cx).composer.read(cx).value(),
                    "d\nt\nmore"
                );
            }
            fixture.view.update(cx, |view, cx| view.save_title(cx));
        })
        .expect("collapse uses the latest setting");
        cx.run_until_parked();
        if mode == composer::Mode::Send {
            let Command::Prompt { text, .. } = fixture
                .commands
                .try_recv()
                .expect("latest Enter setting sends")
            else {
                panic!("expected prompt");
            };
            assert_eq!(text, "draft\nmore");
        }
        assert!(fixture.commands.try_recv().is_err());
    }
}

#[gpui_kit::test]
fn composer_expansion_shortcut_does_not_expand_while_renaming(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("retained draft", window, cx)
            });
            view.begin_rename(window, cx);
        });
        window.press("alt-shift-escape", cx);
        let view = fixture.view.read(cx);
        assert!(!view.composer_expanded);
        assert_eq!(view.composer.read(cx).value(), "retained draft");
        assert!(view.title_edit.is_none());
        assert!(view.composer.read(cx).focus_handle(cx).is_focused(window));
    })
    .expect("expansion shortcut belongs to the message editor");
}

#[gpui_kit::test]
fn compact_composer_expansion_grows_and_caps_without_discarding_lines(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.compact = true;
            view.output_open = false;
            view.composer
                .update(cx, |input, cx| input.set_value("draft", window, cx));
            view.resize_window(window, cx);
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let collapsed_height = window.find("composer-editor").bounds().size.height;
        window.click("expand-message-editor", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        let expanded_height = window.find("composer-editor").bounds().size.height;
        let line_height = fixture
            .view
            .read(cx)
            .composer
            .read(cx)
            .line_height()
            .expect("line height");
        assert!(expanded_height >= line_height * 12.);
        assert!(expanded_height > collapsed_height);
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("line\n".repeat(25), window, cx)
            });
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let limited_height = window.find("composer-editor").bounds().size.height;
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("line\n".repeat(50), window, cx)
            });
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("composer-editor").bounds().size.height,
            limited_height
        );
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "line\n".repeat(50)
        );
        assert_eq!(fixture.view.read(cx).composer_rows, 51);
        window.click("expand-message-editor", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        assert!(window.find("composer-editor").bounds().size.height < expanded_height);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "line\n".repeat(50)
        );
    })
    .expect("compact expansion geometry");
}

#[gpui_kit::test]
fn compact_expanded_composer_keeps_output_and_controls_inside_the_window(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.compact = true;
            view.output_open = true;
            view.composer.update(cx, |input, cx| {
                input.set_value("line\n".repeat(50), window, cx);
            });
            view.resize_window(window, cx);
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        window.click("expand-message-editor", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(window.find("agent-output").bounds().size.height > px(50.));
        for id in ["composer-editor", "composer-toolbar"] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.bottom() <= window.bounds().bottom(),
                "{id}: {bounds:?}; window: {:?}",
                window.bounds()
            );
        }
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "line\n".repeat(50)
        );
    })
    .expect("expanded editor fits with compact output");
}

#[gpui_kit::test]
fn expanded_composer_remeasures_for_short_and_tall_full_windows(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.epoch,
                    event: Event::new(
                        meta.id,
                        Cursor::from_raw(1),
                        EventKind::Acp(basr::acp::SessionUpdate::UserMessageChunk(
                            basr::acp::ContentChunk::new("existing conversation".into()),
                        )),
                    ),
                },
                window,
                cx,
            );
            view.composer.update(cx, |input, cx| {
                input.set_value("line\n".repeat(50), window, cx);
            });
        });
        window.press("alt-shift-escape", cx);
        let mut heights = Vec::new();
        for height in [400., 800., 400.] {
            window.resize(size(px(640.), px(height)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            window.render_frame(cx);
            let editor = window.find("composer-editor").bounds();
            let header = window.find("thread-header").bounds();
            let toolbar = window.find("composer-toolbar").bounds();
            assert!(
                editor.top() >= header.bottom() + px(50.),
                "editor: {editor:?}; header: {header:?}; window: {:?}",
                window.bounds()
            );
            assert!(toolbar.bottom() <= window.bounds().bottom());
            heights.push(editor.size.height);
        }
        assert!(heights[0] < heights[1]);
        assert_eq!(heights[0], heights[2]);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "line\n".repeat(50)
        );
    })
    .expect("expanded full window geometry");
}
