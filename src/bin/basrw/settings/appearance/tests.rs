use super::form::FontChoice;
use super::*;
use crate::theme::preferences::{Background, ThemeMode, ThemeSource};
use gpui_kit::{Entity, TestAppContext, WindowHandle, size, test::TestWindowExt};

struct Fixture {
    view: Entity<AppearancePage>,
    window: WindowHandle<Root>,
    path: std::path::PathBuf,
    _directory: tempfile::TempDir,
}

fn fixture(cx: &mut TestAppContext, preferences: Preferences) -> Fixture {
    let directory = tempfile::tempdir().expect("settings test directory");
    let path = directory.path().join("settings.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&basr::config::Settings {
            appearance: preferences,
            ..Default::default()
        })
        .expect("settings serialize"),
    )
    .expect("initial configuration");
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(None, Some(path.clone()), cx);
    });
    let mut view = None;
    let window = cx.open_window(size(px(750.), px(680.)), |window, cx| {
        let page = cx.new(|cx| AppearancePage::new(theme::Editor::at(path.clone()), window, cx));
        view = Some(page.clone());
        Root::new(page, window, cx)
    });
    Fixture {
        view: view.expect("page created"),
        window,
        path,
        _directory: directory,
    }
}

fn builtin() -> Preferences {
    Preferences {
        theme_source: ThemeSource::Builtin,
        ..Preferences::default()
    }
}

fn read(fixture: &Fixture) -> Preferences {
    basr::config::Store::at(fixture.path.clone())
        .load()
        .expect("valid settings")
        .settings
        .appearance
}

#[gpui_kit::test]
fn text_edits_debounce_flush_and_reject_invalid_values(cx: &mut TestAppContext) {
    let fixture = fixture(cx, builtin());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        fixture.view.update(cx, |page, cx| {
            page.scroll_to = Some(Group::Fonts.index());
            cx.notify();
        });
        window.render_frame(cx);
        assert!(
            window.find("appearance-content-size").visible(),
            "the sidebar scrolls the fonts section into view"
        );
        window.click("appearance-content-size", cx);
        window.input("23", cx);
    })
    .expect("input change");
    assert_eq!(read(&fixture).content_font_size, None);
    cx.executor().run_until_parked();
    cx.executor().advance_clock(DEBOUNCE);
    cx.executor().run_until_parked();
    assert_eq!(read(&fixture).content_font_size, Some(23.));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(theme::content_font_size(cx), px(23.));
        fixture.view.update(cx, |page, cx| {
            page.form
                .content_size
                .update(cx, |input, cx| input.set_value("-2", window, cx));
            page.flush_pending(window, cx);
            assert!(page.message.contains("未应用"));
        });
    })
    .expect("invalid input");
    assert_eq!(read(&fixture).content_font_size, Some(23.));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |page, cx| {
            page.form
                .content_size
                .update(cx, |input, cx| input.set_value("25", window, cx));
            page.flush_pending(window, cx);
        });
        assert_eq!(theme::content_font_size(cx), px(25.));
    })
    .expect("close-style flush");
    assert_eq!(read(&fixture).content_font_size, Some(25.));
}

#[gpui_kit::test]
fn fonts_use_default_and_named_choices_in_one_control(cx: &mut TestAppContext) {
    let mut preferences = builtin();
    preferences.ui_font_family = Some("Configured but not installed".into());
    let fixture = fixture(cx, preferences);
    let select = cx.update(|cx| fixture.view.read(cx).form.ui_font.clone());
    for font in [
        FontChoice::Default,
        FontChoice::Named("Configured but not installed".into()),
    ] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            select.update(cx, |select, cx| {
                select.set_selected_value(&font, window, cx);
                cx.emit(SelectEvent::Confirm(Some(font.clone())));
            });
        })
        .expect("choose font");
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                fixture.view.read(cx).form.ui_font.entity_id(),
                select.entity_id()
            );
        })
        .expect("self-save does not replace the focused control");
        let expected = match &font {
            FontChoice::Default => None,
            FontChoice::Named(name) => Some(name.clone()),
        };
        assert_eq!(read(&fixture).ui_font_family, expected);
    }
}

#[gpui_kit::test]
fn font_default_can_be_chosen_with_the_real_searchable_dropdown(cx: &mut TestAppContext) {
    let fixture = fixture(
        cx,
        Preferences {
            ui_font_family: Some("Only configured in this test".into()),
            ..builtin()
        },
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let select = fixture.view.read(cx).form.ui_font.clone();
        select.update(cx, |select, cx| select.focus(window, cx));
        window.press("down", cx);
    })
    .expect("open font dropdown");
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("Default", cx);
    })
    .expect("search font dropdown");
    cx.executor().run_until_parked();
    // The component keeps search busy for 100ms to avoid flickering.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    cx.executor().run_until_parked();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .expect("confirm default choice");
    cx.executor().run_until_parked();
    assert_eq!(read(&fixture).ui_font_family, None);
}

#[gpui_kit::test]
fn external_reload_updates_clean_controls_but_preserves_dirty_conflicts(cx: &mut TestAppContext) {
    let fixture = fixture(cx, builtin());
    let external = Preferences {
        ui_font_size: Some(21.),
        ..builtin()
    };
    std::fs::write(
        &fixture.path,
        serde_json::to_vec(&serde_json::json!({"appearance": external})).expect("serialize"),
    )
    .expect("external edit");
    cx.update(|cx| theme::init(None, Some(fixture.path.clone()), cx));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).form.ui_size.read(cx).value(), "21");
        fixture.view.update(cx, |page, cx| {
            page.form
                .ui_size
                .update(cx, |input, cx| input.set_value("24", window, cx));
        });
    })
    .expect("external update rendered");
    let newer = Preferences {
        ui_font_size: Some(27.),
        ..builtin()
    };
    std::fs::write(
        &fixture.path,
        serde_json::to_vec(&serde_json::json!({"appearance": newer})).expect("serialize"),
    )
    .expect("concurrent edit");
    cx.update(|cx| theme::init(None, Some(fixture.path.clone()), cx));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        fixture.view.update(cx, |page, cx| {
            assert_eq!(page.form.ui_size.read(cx).value(), "24");
            page.flush_pending(window, cx);
            assert!(page.message.contains("外部修改"));
        });
        assert_eq!(Theme::global(cx).font_size, px(27.));
    })
    .expect("conflict preserves local input and external configuration");
    assert_eq!(read(&fixture).ui_font_size, Some(27.));
}

#[gpui_kit::test]
fn all_fields_round_trip_and_default_is_applied_immediately(cx: &mut TestAppContext) {
    let expected = Preferences {
        ui_font_family: Some("UI custom".into()),
        mono_font_family: Some("Mono custom".into()),
        font_fallbacks: Some(vec!["Fallback A".into()]),
        ui_font_size: Some(17.),
        content_font_size: Some(19.),
        content_line_height: Some(1.8),
        content_opacity: Some(0.8),
        theme_file: Some("theme.json".into()),
        theme_source: ThemeSource::Builtin,
        theme_mode: ThemeMode::Dark,
        background: Background::Opaque,
    };
    let fixture = fixture(cx, expected.clone());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert_eq!(fixture.view.read(cx).candidate(cx).expect("form"), expected);
        fixture.view.update(cx, |page, cx| {
            page.form
                .fallbacks
                .update(cx, |input, cx| input.set_value("[]", window, cx));
            page.flush_pending(window, cx);
            assert_eq!(
                page.candidate(cx).expect("empty fallback").font_fallbacks,
                Some(vec![])
            );
        });
        window.render_frame(cx);
        window.click("settings-reset", cx);
    })
    .expect("reset control");
    assert_eq!(read(&fixture), Preferences::default());
}
