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
            let left = window.find("add-attachment").bounds().left();
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
                window.find("add-attachment").bounds().left(),
                left,
                "alignment follows the text origin rather than the caret"
            );
        })
        .expect("aligned controls");
    }
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
