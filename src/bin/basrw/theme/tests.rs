use gpui_kit::TestAppContext;

use super::*;

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
    let path = directory.path().join("appearance.json");
    std::fs::write(&path, r#"{"theme_source":"builtin","ui_font_size":16}"#)
        .expect("initial appearance");
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(None, Some(path.clone()), cx);
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
    std::fs::write(&path, r#"{"theme_source":"builtin","ui_font_size":22}"#)
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
    std::fs::write(&path, r#"{"theme_source":"builtin","ui_font_size":25}"#)
        .expect("update with only settings left");
    cx.executor().advance_clock(REFRESH_INTERVAL);
    cx.executor().run_until_parked();
    cx.update_window(second.into(), |_, window, cx| window.render_frame(cx))
        .expect("remaining window still receives changes");
    assert_eq!(second_size.get(), 25.);
    std::fs::write(&path, r#"{"ui_font_size":0}"#).expect("invalid external update");
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
