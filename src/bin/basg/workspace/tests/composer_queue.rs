use super::*;

#[gpui_kit::test]
fn composer_context_menu_contains_only_working_text_actions(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("copy this", window, cx);
                input.set_selected_range(0..9, cx);
                input.focus(window, cx);
            });
        });
        window.render_frame(cx);
        assert_eq!(
            fixture
                .view
                .read(cx)
                .composer
                .input
                .read(cx)
                .selected_range(),
            0..9
        );
        use gpui_kit::InputEvent;
        let position = fixture
            .view
            .read(cx)
            .composer
            .input
            .read(cx)
            .range_to_bounds(&(0..9))
            .expect("selected text")
            .center();
        window.simulate_mouse_move(position, cx);
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                button: gpui_kit::MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        assert_eq!(
            fixture
                .view
                .read(cx)
                .composer
                .input
                .read(cx)
                .selected_range(),
            0..9
        );
    })
    .expect("composer right-click");
    cx.run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        for (index, label) in ["edit-cut", "edit-copy", "edit-paste"].iter().enumerate() {
            let expected = match *label {
                "edit-cut" => crate::i18n::tr!("edit-cut"),
                "edit-copy" => crate::i18n::tr!("edit-copy"),
                _ => crate::i18n::tr!("edit-paste"),
            };
            assert_eq!(
                window.within("popup-menu").find(index).label(),
                Some(expected)
            );
        }
        window.within("popup-menu").click(1usize, cx);
    })
    .expect("copy action");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("copy this")
        )
    });
}

#[gpui_kit::test]
fn new_workspace_uses_the_persisted_enter_behavior(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let editor = theme::Editor::current(cx);
        let mut document = editor.load().expect("settings document");
        let mut preferences = document.preferences.clone();
        preferences.composer_enter_behavior = composer::Mode::Newline;
        editor
            .save(&mut document, preferences, window, cx)
            .expect("save Enter behavior");
    })
    .expect("persist before creating another workspace");
    let (commands, _commands) = mpsc::unbounded_channel();
    let (_updates, events) = mpsc::channel(EVENT_BATCH);
    let mut view = None;
    let window = cx.open_window(size(px(1120.), px(800.)), |window, cx| {
        let workspace = cx.new(|cx| {
            Workspace::with_connection(
                Connection::for_test(commands),
                events,
                "127.0.0.1:7337".parse().expect("test address"),
                PathBuf::from("/tmp"),
                false,
                window,
                cx,
            )
        });
        view = Some(workspace.clone());
        Root::new(workspace, window, cx)
    });
    let view = view.expect("new workspace");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("新草稿", cx);
        window.press("enter", cx);
        assert_eq!(view.read(cx).composer.input.read(cx).value(), "新草稿\n");
        assert_eq!(view.read(cx).composer.mode, composer::Mode::Newline);
    })
    .expect("saved preference controls native Enter at startup");
}

#[gpui_kit::test]
fn external_enter_preference_changes_preserve_draft_selection_and_rename_focus(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    let original = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.composer.input.update(cx, |input, cx| {
                    input.set_value("保留原草稿\n另一行", window, cx);
                    input.set_selected_range(0..0, cx);
                });
                view.begin_rename(window, cx);
            });
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert!(
                view.renaming
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            view.composer.input.read(cx).value().to_string()
        })
        .expect("keep rename focused");
    cx.update(theme::watch);
    cx.executor().run_until_parked();
    for behavior in [composer::Mode::Newline, composer::Mode::Send] {
        let mut settings = bas::config::Store::at(fixture._appearance.path().join("settings.json"))
            .load()
            .expect("appearance settings")
            .settings;
        settings.appearance.composer_enter_behavior = behavior;
        std::fs::write(
            fixture._appearance.path().join("settings.json"),
            serde_json::to_vec(&settings).expect("settings JSON"),
        )
        .expect("external Enter preference");
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.executor().run_until_parked();
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert_eq!(view.composer.mode, behavior);
            assert_eq!(view.composer.input.read(cx).value(), original);
            assert_eq!(view.composer.input.read(cx).selected_range(), 0..0);
            assert!(
                view.renaming
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert!(window.try_find("composer-input-mode").is_none());
        })
        .expect("external Enter preference applies without focusing the composer");
    }
}

#[gpui_kit::test]
fn composer_send_shortcut_does_not_send_a_draft_while_renaming(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    attach(&fixture, session(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("尚未发送的草稿", window, cx)
            });
            view.begin_rename(window, cx);
        });
        window.render_frame(cx);
        window.press("ctrl-enter", cx);
    })
    .expect("rename window");
    while let Ok(command) = fixture.commands.try_recv() {
        assert!(!matches!(command, Command::Prompt { .. }));
    }
    cx.update(|cx| {
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "尚未发送的草稿"
        );
    });
}

#[gpui_kit::test]
fn multiline_composer_keeps_enter_local_and_sends_with_control_enter(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    let session = session();
    attach(&fixture, session, cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("composer-input-mode").is_none());
        window.input("第一行", cx);
        let editor = theme::Editor::current(cx);
        let mut document = editor.load().expect("appearance settings");
        let mut preferences = document.preferences.clone();
        preferences.composer_enter_behavior = composer::Mode::Newline;
        editor
            .save(&mut document, preferences, window, cx)
            .expect("persist multiline input");
    })
    .expect("save Enter behavior");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
        window.input("第二行", cx);
        assert_eq!(
            fixture.view.read(cx).composer.input.read(cx).value(),
            "第一行\n第二行"
        );
        assert!(fixture.commands.try_recv().is_err());
        window.press("ctrl-enter", cx);
    })
    .expect("composer window");
    let Command::Prompt { submission, .. } =
        fixture.commands.try_recv().expect("control enter sends")
    else {
        panic!("expected prompt");
    };
    assert_eq!(submission::text(&submission), "第一行\n第二行");
}

#[gpui_kit::test]
fn pending_panel_edits_arbitrary_messages_and_withdrawing_restores_the_composition(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let mut meta = session();
    meta.status = SessionStatus::Working;
    attach(&fixture, meta.clone(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for text in ["A", "B", "C"] {
                view.enqueue_draft(meta.id, text.into(), window, cx);
            }
            view.composer
                .input
                .update(cx, |input, cx| input.set_value("composition", window, cx));
        });
        window.render_frame(cx);
        let panel = window.find("pending-messages-panel").bounds();
        assert!(panel.bottom() <= window.find("conversation-status").bounds().top());
        window.click("edit-pending-message-1", cx);
        assert_eq!(fixture.view.read(cx).composer.input.read(cx).value(), "B");
        window.render_frame(cx);
        window.click("withdraw-pending-message-0", cx);
        window.render_frame(cx);
        window.click("withdraw-pending-message-0", cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.composer.input.read(cx).value(), "composition");
        assert_eq!(
            view.sessions.outboxes[&meta.id]
                .front()
                .expect("remaining queued message")
                .text,
            "C"
        );
        assert_eq!(view.sessions.outboxes[&meta.id].len(), 1);
    })
    .expect("pending panel window");
}

#[gpui_kit::test]
fn immediate_queue_send_waits_for_cancel_and_completion_and_keeps_other_drafts_paused(
    cx: &mut TestAppContext,
) {
    let mut fixture = fixture(cx);
    let previous = bas::model::TurnId::new();
    let mut meta = session();
    meta.status = SessionStatus::Working;
    meta.active_turn = Some(previous);
    attach(&fixture, meta.clone(), cx);
    while fixture.commands.try_recv().is_ok() {}
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.enqueue_draft(meta.id, "A".into(), window, cx);
            view.enqueue_draft(meta.id, "B".into(), window, cx);
            view.sessions.outboxes.get_mut(&meta.id).expect("queue").pause();
        });
        window.render_frame(cx);
        window.click("send-pending-message-1", cx);
        assert!(matches!(fixture.commands.try_recv().expect("interrupt command"), Command::Cancel(id) if id == meta.id));
        assert!(fixture.commands.try_recv().is_err());
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Cancelled { session: meta.id, result: Ok(()) }, window, cx);
        });
        assert!(fixture.commands.try_recv().is_err(), "cancel acknowledgement is not turn completion");
        fixture.view.update(cx, |view, cx| {
            view.update(Update::Event {
                epoch: view.selected.epoch,
                event: Event::new(meta.id, Cursor::from_raw(1), EventKind::TurnFinished {
                    turn: previous,
                    result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::Cancelled)),
                }),
            }, window, cx);
        });
        let command = fixture.commands.try_recv().expect("send selected draft");
        let Command::Prompt { submission, .. } = &command else {
            panic!("expected selected prompt");
        };
        assert_eq!(submission::text(submission), "B");
        let submission = submission::payload(command);
        let accepted = bas::model::TurnReceipt { turn: submission.turn, cursor: Cursor::from_raw(2) };
        fixture.view.update(cx, |view, cx| {
            view.update(submission::accepted_at(meta.id, &submission, 2), window, cx);
            view.update(Update::Event {
                epoch: view.selected.epoch,
                event: Event::new(meta.id, Cursor::from_raw(3), EventKind::TurnFinished {
                    turn: accepted.turn,
                    result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
                }),
            }, window, cx);
        });
        assert!(fixture.commands.try_recv().is_err());
        assert_eq!(fixture.view.read(cx).sessions.outboxes[&meta.id].front().expect("paused A").text, "A");
    }).expect("immediate send window");
}

#[gpui_kit::test]
fn finishing_before_cancel_acknowledgement_keeps_the_immediate_draft_until_success(
    cx: &mut TestAppContext,
) {
    for succeeded in [true, false] {
        let mut fixture = fixture(cx);
        let previous = bas::model::TurnId::new();
        let mut meta = session();
        meta.status = SessionStatus::Working;
        meta.active_turn = Some(previous);
        attach(&fixture, meta.clone(), cx);
        while fixture.commands.try_recv().is_ok() {}
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.enqueue_draft(meta.id, "原队列消息".into(), window, cx);
                view.sessions.outboxes.get_mut(&meta.id).expect("queue").pause();
                view.composer.input.update(cx, |input, cx| input.set_value("立即发送的新消息", window, cx));
            });
            window.render_frame(cx);
            window.click("send-immediately", cx);
            assert!(matches!(fixture.commands.try_recv().expect("cancel current turn"), Command::Cancel(id) if id == meta.id));
            fixture.view.update(cx, |view, cx| {
                view.update(Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(meta.id, Cursor::from_raw(1), EventKind::TurnFinished {
                        turn: previous,
                        result: Ok(bas::acp::PromptResponse::new(bas::acp::StopReason::EndTurn)),
                    }),
                }, window, cx);
            });
            assert!(fixture.commands.try_recv().is_err(), "a finished turn cannot bypass cancel acknowledgement");
            fixture.view.update(cx, |view, cx| {
                view.update(Update::Cancelled {
                    session: meta.id,
                    result: if succeeded { Ok(()) } else { Err("cancel RPC failed".into()) },
                }, window, cx);
            });
            if succeeded {
                let Command::Prompt { submission, .. } = fixture.commands.try_recv().expect("acknowledged immediate send") else {
                    panic!("expected immediate prompt");
                };
                assert_eq!(submission::text(&submission), "立即发送的新消息");
                assert_eq!(fixture.view.read(cx).sessions.outboxes[&meta.id].front().expect("paused original message").text, "原队列消息");
            } else {
                assert!(fixture.commands.try_recv().is_err(), "failed cancellation keeps the draft queued");
                let view = fixture.view.read(cx);
                assert!(view.sessions.outboxes[&meta.id].paused());
                assert_eq!(view.sessions.outboxes[&meta.id].front().expect("failed immediate draft").text, "立即发送的新消息");
                assert_eq!(view.sessions.outboxes[&meta.id].len(), 2);
                assert!(view.sessions.errors.get(&meta.id).expect("cancellation error").contains("cancel RPC failed"));
            }
        }).expect("early completion window");
    }
}

#[gpui_kit::test]
fn clipboard_keeps_text_alongside_images_and_when_the_agent_rejects_images(
    cx: &mut TestAppContext,
) {
    for image_supported in [true, false] {
        let fixture = fixture(cx);
        let mut meta = session();
        meta.initialization = Some(
            bas::acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
                .agent_capabilities(bas::acp::AgentCapabilities::new().prompt_capabilities(
                    bas::acp::PromptCapabilities::new().image(image_supported),
                )),
        );
        attach(&fixture, meta, cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let image = gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Png,
                b"\x89PNG\r\n\x1a\n".to_vec(),
            );
            cx.write_to_clipboard(ClipboardItem {
                entries: vec![
                    gpui_kit::ClipboardEntry::Image(image),
                    gpui_kit::ClipboardEntry::String(gpui_kit::ClipboardString::new(
                        "图片说明".into(),
                    )),
                ],
            });
            window.press("ctrl-v", cx);
            let view = fixture.view.read(cx);
            let text = view.composer.input.read(cx).value();
            assert!(text.ends_with("图片说明"));
            assert_eq!(
                crate::attachment::mentions(&text, &view.composer.attachments).len(),
                usize::from(image_supported)
            );
            if !image_supported {
                assert_eq!(text, "图片说明");
                assert_eq!(
                    view.selected.error.as_deref(),
                    Some("请改用文本描述图片内容。")
                );
            }
        })
        .expect("mixed clipboard window");
    }
}
