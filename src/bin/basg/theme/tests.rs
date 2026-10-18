use gpui_kit::TestAppContext;

use super::*;

#[gpui_kit::test]
fn native_windows_share_root_typography_and_live_updates(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext, Render, div, prelude::*, size, test::TestWindowExt};

    struct Probe(Rc<std::cell::RefCell<Option<gpui_kit::TextStyle>>>);
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let metrics = self.0.clone();
            div().child(
                gpui_kit::canvas(
                    move |_, window, _| {
                        *metrics.borrow_mut() = Some(window.text_style());
                    },
                    |_, _, _, _| {},
                )
                .size_full(),
            )
        }
    }

    let directory = tempfile::tempdir().expect("isolated settings");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(directory.path().join("settings.json")), cx);
        init(None, cx);
    });
    let metrics = Rc::new(std::cell::RefCell::new(None));
    let main = cx.open_window(size(px(960.), px(720.)), |window, cx| {
        let view = cx.new(|_| Probe(metrics.clone()));
        window_root(view, window, cx)
    });
    let windows = cx.update(|cx| {
        vec![
            main,
            crate::settings::window::open(cx).expect("settings"),
            crate::application::open_about(cx).expect("about"),
            crate::application::open_quit(cx).expect("quit"),
            crate::application::open_message("Title", "Message", cx).expect("message"),
            crate::application::open_confirmation(
                crate::application::Confirmation {
                    title: "Confirm".into(),
                    message: "Message".into(),
                    confirm_label: "OK".into(),
                    destructive: false,
                },
                |_| {},
                cx,
            )
            .expect("confirmation"),
        ]
    });
    for ui_size in [15., 22.] {
        cx.update(|cx| {
            let effective = preferences::resolve(
                &Preferences {
                    ui_font_size: Some(ui_size),
                    content_font_size: Some(19.),
                    ..Default::default()
                },
                source::fallback(true),
            )
            .expect("valid typography");
            apply(effective, None, cx);
        });
        cx.executor().run_until_parked();
        for handle in &windows {
            cx.update_window((*handle).into(), |_, window, cx| {
                window.render_frame(cx);
                if window.try_find("settings-appearance-text").is_some() {
                    let metrics = metrics.borrow();
                    let metrics = metrics.as_ref().expect("workspace typography was painted");
                    for id in [
                        "settings-appearance-text",
                        "settings-theme-text",
                        "settings-fonts-text",
                        "settings-layout-text",
                        "appearance-language-label",
                        "appearance-source-label",
                    ] {
                        // Upstream buttons use one-em line height; body text uses the default.
                        let expected = if id.starts_with("settings-") {
                            metrics.font_size.to_pixels(px(ui_size)).round()
                        } else {
                            metrics.line_height_in_pixels(px(ui_size))
                        };
                        let height = window.find(id).bounds().size.height;
                        assert!(
                            (f32::from(height - expected)).abs() <= 1.,
                            "{id}: rendered height {height:?}, workspace text height {expected:?}"
                        );
                    }
                }
            })
            .expect("render open window");
            cx.update(|cx| {
                handle.update(cx, |root, window, cx| {
                    assert_eq!(window.rem_size(), px(ui_size));
                    let expected = font_style(cx);
                    assert_eq!(root.style().text.font_size, expected.text.font_size);
                    assert_eq!(
                        root.style().text.font_fallbacks,
                        expected.text.font_fallbacks
                    );
                })
            })
            .expect("open window follows shared typography");
        }
        let actual = metrics.borrow();
        let actual = actual.as_ref().expect("root typography was painted");
        assert_eq!(
            actual.font_size.to_pixels(px(ui_size)),
            crate::ui::SMALL.to_pixels(px(ui_size))
        );
        cx.update(|cx| assert_eq!(content_font_size(cx), px(19.)));
    }
}

#[gpui_kit::test]
fn syntax_defaults_follow_theme_switches_and_preserve_custom_colors(cx: &mut TestAppContext) {
    use gpui_kit::component::highlighter::{HighlightTheme, HighlightThemeStyle};
    cx.update(|cx| {
        gpui_kit::init(cx);
        for dark in [true, false, true] {
            let expected = if dark {
                HighlightTheme::default_dark()
            } else {
                HighlightTheme::default_light()
            };
            let resolved = source::fallback(dark);
            let effective = preferences::resolve(&Preferences::default(), resolved)
                .expect("built-in appearance");
            apply(effective, None, cx);
            let actual = &Theme::global(cx).highlight_theme;
            assert_eq!(actual.appearance.is_dark(), dark);
            assert_eq!(actual.style.syntax, expected.style.syntax);
        }
        let mut resolved = source::fallback(true);
        let editor_background = gpui_kit::rgb(0x123456).into();
        resolved.config.highlight = Some(HighlightThemeStyle {
            editor_background: Some(editor_background),
            ..Default::default()
        });
        let effective = preferences::resolve(&Preferences::default(), resolved)
            .expect("theme with editor colors");
        apply(effective, None, cx);
        let actual = &Theme::global(cx).highlight_theme;
        assert_eq!(actual.style.editor_background, Some(editor_background));
        assert_eq!(
            actual.style.syntax,
            HighlightTheme::default_dark().style.syntax
        );

        let mut resolved = source::fallback(true);
        let custom: HighlightThemeStyle = serde_json::from_value(serde_json::json!({
            "syntax": {"function": {"color": "#f1c678"}}, "editor.background": "#123456"
        }))
        .expect("custom syntax palette");
        resolved.config.highlight = Some(custom.clone());
        let effective =
            preferences::resolve(&Preferences::default(), resolved).expect("custom appearance");
        apply(effective, None, cx);
        assert_eq!(Theme::global(cx).highlight_theme.style, custom);
    });
}

#[gpui_kit::test]
fn application_watcher_survives_window_closure_and_updates_all_views(cx: &mut TestAppContext) {
    use gpui_kit::component::Root;
    use gpui_kit::{
        AppContext, Context, IntoElement, Render, Window, div, prelude::*, size,
        test::TestWindowExt,
    };

    struct Probe(std::rc::Rc<std::cell::Cell<f32>>);
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.set(f32::from(Theme::global(cx).font_size));
            div().child("theme probe")
        }
    }

    let directory = tempfile::tempdir().expect("isolated preferences");
    let path = directory.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"appearance":{"theme_source":"builtin","ui_font_size":16}}"#,
    )
    .expect("initial appearance");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        init(None, cx);
        watch(cx);
        watch(cx); // Repeated callers share a single application watcher.
    });
    let first_size = std::rc::Rc::new(std::cell::Cell::new(0.));
    let second_size = std::rc::Rc::new(std::cell::Cell::new(0.));
    let first = cx.open_window(size(px(500.), px(400.)), |window, cx| {
        Root::new(cx.new(|_| Probe(first_size.clone())), window, cx)
    });
    let second = cx.open_window(size(px(500.), px(400.)), |window, cx| {
        Root::new(cx.new(|_| Probe(second_size.clone())), window, cx)
    });
    cx.executor().run_until_parked();
    std::fs::write(
        &path,
        r#"{"appearance":{"theme_source":"builtin","ui_font_size":22}}"#,
    )
    .expect("external update");
    cx.executor().advance_clock(REFRESH_INTERVAL);
    cx.executor().run_until_parked();
    for handle in [first, second] {
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .expect("render both windows");
    }
    assert_eq!(first_size.get(), 22.);
    assert_eq!(second_size.get(), 22.);
    cx.update_window(first.into(), |_, window, _| window.remove_window())
        .expect("close original window");
    std::fs::write(
        &path,
        r#"{"appearance":{"theme_source":"builtin","ui_font_size":25}}"#,
    )
    .expect("update with only settings left");
    cx.executor().advance_clock(REFRESH_INTERVAL);
    cx.executor().run_until_parked();
    cx.update_window(second.into(), |_, window, cx| window.render_frame(cx))
        .expect("remaining window still receives changes");
    assert_eq!(second_size.get(), 25.);
    std::fs::write(&path, r#"{"appearance":{"ui_font_size":0}}"#).expect("invalid external update");
    cx.executor().advance_clock(REFRESH_INTERVAL);
    cx.executor().run_until_parked();
    cx.update(|cx| assert_eq!(Theme::global(cx).font_size, px(25.)));
}

#[gpui_kit::test]
fn preference_updates_reach_render_metrics_without_a_mode_change(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        init_default(cx);
        for ui_size in [18., 22.] {
            let preferences = Preferences {
                ui_font_size: Some(ui_size),
                content_font_size: Some(17.),
                content_line_height: Some(2.),
                content_opacity: Some(0.8),
                background: preferences::Background::Opaque,
                ..Preferences::default()
            };
            let effective = preferences::resolve(&preferences, source::fallback(true))
                .expect("explicit appearance values should be valid");
            apply(effective, None, cx);

            assert_eq!(Theme::global(cx).font_size, px(ui_size));
            assert_eq!(content_font_size(cx), px(17.));
            assert_eq!(content_line_height(cx), 2.);
            assert_eq!(content_background(cx).a, 0.8);
            assert_eq!(
                cx.global::<Appearance>().background,
                WindowBackgroundAppearance::Opaque
            );
        }
    });
}
