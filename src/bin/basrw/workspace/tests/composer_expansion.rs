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
            fixture
                .view
                .update(cx, |view, _| view.editing_title = false);
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
        assert!(!view.editing_title);
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
    attach(&fixture, session(), cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
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
