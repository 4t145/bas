use super::*;

#[gpui_kit::test]
fn composer_controls_stay_aligned_after_resize_font_change_and_cursor_movement(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    for (width, font_size) in [(600., 14.), (1400., 24.), (600., 24.)] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.resize(size(px(width), px(800.)));
            window.bounds_changed(cx);
            let editor = theme::Editor::current(cx);
            let mut document = editor.load().expect("appearance document");
            let mut preferences = document.preferences.clone();
            preferences.content_font_size = Some(font_size);
            editor
                .save(&mut document, preferences, window, cx)
                .expect("apply content font");
            fixture.view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value("中文 emoji 👩‍💻 English", window, cx);
                    input.set_selected_range(0..0, cx);
                });
            });
            window.render_frame(cx);
        })
        .expect("paint editor geometry");
        cx.run_until_parked();
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let attachment = window.find("add-attachment").bounds();
            let left = window.find("agent-picker-icon").bounds().left();
            let panel = window.find("composer-panel").bounds();
            let body = window.find("thread-body").bounds();
            let status = window.find("conversation-status").bounds();
            assert_eq!(panel.left() - body.left(), px(chat::THREAD_BODY_PADDING));
            assert_eq!(body.right() - panel.right(), px(24.));
            assert_eq!(status.left(), panel.left());
            assert_eq!(status.right(), panel.right());
            let editor_bounds = window.find("composer-editor").bounds();
            let expand = window.find("expand-message-editor").bounds();
            let send = window.find("send-message").bounds();
            assert_eq!(expand.center().x, send.center().x);
            assert!(
                expand.top() >= editor_bounds.top() && expand.bottom() <= editor_bounds.bottom()
            );
            assert!(panel.right() - send.right() <= px(6.));
            assert_eq!(
                attachment.center().x - window.find("agent-picker-icon").bounds().size.width / 2.,
                left,
                "the upper agent icon and lower attachment icon share the text origin"
            );
            assert!(window.try_find("composer-header").is_none());
            let composer = fixture.view.read(cx).composer.clone();
            assert_eq!(
                composer
                    .read(cx)
                    .cursor_layout()
                    .expect("first caret")
                    .0
                    .left(),
                left
            );
            let end = composer.read(cx).value().len();
            composer.update(cx, |input, cx| input.set_selected_range(end..end, cx));
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("agent-picker-icon").bounds().left(),
                left,
                "alignment follows the text origin rather than the caret"
            );
        })
        .expect("aligned controls");
    }
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer
                .update(cx, |input, cx| input.set_selected_range(0..0, cx));
        });
        window.press("ctrl-shift-m", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let icon_width = window.find("agent-picker-icon").bounds().size.width;
        let upper_icon_left = window.find("toggle-output").bounds().center().x - icon_width / 2.;
        let lower_icon_left = window.find("add-attachment").bounds().center().x - icon_width / 2.;
        assert_eq!(upper_icon_left, lower_icon_left);
        assert_eq!(
            fixture
                .view
                .read(cx)
                .composer
                .read(cx)
                .cursor_layout()
                .expect("compact first caret")
                .0
                .left(),
            upper_icon_left,
            "compact text aligns with the output toggle and attachment icons"
        );
    })
    .expect("compact icon alignment");
}

#[gpui_kit::test]
fn full_body_uses_shared_padding_for_initialization_history_and_composer(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    let message_id = cx
        .update_window(fixture.window.into(), |_, window, cx| {
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
                                basr::acp::ContentChunk::new("Agent initialization output".into()),
                            )),
                        )],
                    },
                    window,
                    cx,
                );
                view.stream = StreamState::Live;
                view.conversation
                    .as_ref()
                    .expect("initialized conversation")
                    .items[0]
                    .id()
            })
        })
        .expect("prepare initialization output");
    for started in [false, true] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            if started {
                fixture.view.update(cx, |view, cx| {
                    view.update(
                        Update::Event {
                            epoch: view.epoch,
                            event: Event::new(
                                meta.id,
                                Cursor::from_raw(2),
                                EventKind::Acp(basr::acp::SessionUpdate::UserMessageChunk(
                                    basr::acp::ContentChunk::new("first user prompt".into()),
                                )),
                            ),
                        },
                        window,
                        cx,
                    );
                    view.composer_expanded = false;
                });
            }
            for width in [600., 1500.] {
                window.resize(size(px(width), px(800.)));
                window.bounds_changed(cx);
                window.render_frame(cx);
                window.render_frame(cx);
                let body = window.find("thread-body").bounds();
                let left = body.left() + px(chat::THREAD_BODY_PADDING);
                let right = body.right() - px(24.);
                let region = if started {
                    "conversation-timeline"
                } else {
                    "agent-initialization-messages"
                };
                for id in [
                    "conversation-status",
                    "composer-panel",
                    region,
                    message_id.as_str(),
                ] {
                    let bounds = window.find(id.to_owned()).bounds();
                    assert_eq!(bounds.left(), left, "left edge of {id}");
                    assert_eq!(bounds.right(), right, "right edge of {id}");
                }
                let content = window
                    .find(format!("message-content-{message_id}"))
                    .bounds();
                assert_eq!(
                    content.center().x,
                    window.find(message_id.clone()).bounds().center().x
                );
                assert_eq!(
                    content.size.width,
                    px(chat::CONTENT_WIDTH).min(right - left),
                    "message content keeps its width cap within the shared padding"
                );
            }
        })
        .expect("consistent full-window content edges across initialization and history");
    }
}

#[gpui_kit::test]
fn full_composer_reserves_four_rows_then_grows_and_compact_keeps_one(cx: &mut TestAppContext) {
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
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        });
        window.render_frame(cx);
        window.render_frame(cx);
        let empty_height = window.find("composer-editor").bounds().size.height;
        let line_height = fixture
            .view
            .read(cx)
            .composer
            .read(cx)
            .line_height()
            .expect("line height");
        assert!(!fixture.view.read(cx).composer_expanded);
        assert!(empty_height >= line_height * 4. && empty_height < line_height * 5.);
        for draft in ["short draft 😀", "one\ntwo\nthree\nfour"] {
            fixture.view.update(cx, |view, cx| {
                view.composer.update(cx, |input, cx| {
                    input.set_value(draft, window, cx);
                    input.set_selected_range(draft.len()..draft.len(), cx);
                });
            });
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("composer-editor").bounds().size.height,
                empty_height
            );
        }
        window.press("shift-enter", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(window.find("composer-editor").bounds().size.height > empty_height);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "one\ntwo\nthree\nfour\n"
        );
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("preserved draft 😀", window, cx);
                input.set_selected_range(0..9, cx);
            });
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("composer-editor").bounds().size.height,
            empty_height
        );
        window.press("ctrl-shift-m", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let compact_height = window.find("composer-editor").bounds().size.height;
        assert!(compact_height >= line_height && compact_height < line_height * 2.);
        assert!(compact_height < empty_height);
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "preserved draft 😀"
        );
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).selected_range(),
            0..9
        );
        window.press("ctrl-shift-m", cx);
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("composer-editor").bounds().size.height,
            empty_height
        );
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "preserved draft 😀"
        );
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).selected_range(),
            0..9
        );
    })
    .expect("default rows, growth and actual mode switches preserve the draft");
}

#[gpui_kit::test]
fn composer_remeasures_wrapping_after_narrow_and_wide_window_resizes(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("中文 emoji 👩‍💻 English words ".repeat(12), window, cx)
            });
        });
        window.resize(size(px(600.), px(800.)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let narrow_height = window.find("composer-editor").bounds().size.height;
        window.resize(size(px(1400.), px(800.)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let wide_height = window.find("composer-editor").bounds().size.height;
        assert!(
            narrow_height > wide_height,
            "narrow={narrow_height:?}, wide={wide_height:?}"
        );
        window.resize(size(px(600.), px(800.)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("composer-editor").bounds().size.height,
            narrow_height
        );
    })
    .expect("composer resize window");
}

#[gpui_kit::test]
fn composer_remeasures_when_content_font_size_changes(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value(
                    "文字混排 with emoji 👩‍💻 and ordinary words ".repeat(8),
                    window,
                    cx,
                )
            });
        });
        window.render_frame(cx);
        window.render_frame(cx);
        let initial_height = window.find("composer-editor").bounds().size.height;
        let initial_line_height = fixture
            .view
            .read(cx)
            .composer
            .read(cx)
            .line_height()
            .expect("painted line height");
        let initial_rows = fixture.view.read(cx).composer_rows;
        let editor = theme::Editor::current(cx);
        let mut document = editor.load().expect("appearance document");
        let mut preferences = document.preferences.clone();
        preferences.content_font_size = Some(24.);
        editor
            .save(&mut document, preferences, window, cx)
            .expect("apply larger content font");
        window.render_frame(cx);
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert!(
            view.composer
                .read(cx)
                .line_height()
                .expect("updated line height")
                > initial_line_height
        );
        assert!(view.composer_rows > initial_rows);
        assert!(window.find("composer-editor").bounds().size.height > initial_height);
    })
    .expect("composer font window");
}

#[gpui_kit::test]
fn compact_composer_caps_height_at_four_rows_while_retaining_additional_lines(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.compact = true;
            view.output_open = false;
            view.composer
                .update(cx, |input, cx| input.set_value("1\n2\n3\n4", window, cx));
            view.resize_window(window, cx);
        });
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let four_row_height = window.find("composer-editor").bounds().size.height;
        fixture.view.update(cx, |view, cx| {
            view.composer.update(cx, |input, cx| {
                input.set_value("1\n2\n3\n4\n5\n6\n7\n8\n", window, cx)
            });
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("composer-editor").bounds().size.height,
            four_row_height
        );
        assert_eq!(
            fixture.view.read(cx).composer.read(cx).value(),
            "1\n2\n3\n4\n5\n6\n7\n8\n"
        );
        assert_eq!(fixture.view.read(cx).composer_rows, 9);
    })
    .expect("compact composer window");
}
