use super::form::FontChoice;
use super::*;
use crate::theme::preferences::{Background, ComposerEnterBehavior, ThemeMode, ThemeSource};
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
        serde_json::to_vec(&bas::config::Settings {
            appearance: preferences,
            ..Default::default()
        })
        .expect("settings serialize"),
    )
    .expect("initial configuration");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path.clone()), cx);
        theme::init(None, cx);
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
    bas::config::Store::at(fixture.path.clone())
        .load()
        .expect("valid settings")
        .settings
        .appearance
}

#[gpui_kit::test]
fn language_selection_saves_and_updates_existing_controls(cx: &mut TestAppContext) {
    if !crate::i18n::isolated_language_test(
        "settings::appearance::tests::language_selection_saves_and_updates_existing_controls",
    ) {
        return;
    }
    use crate::i18n::Language;
    let fixture = fixture(cx, builtin());
    let select = cx.update(|cx| fixture.view.read(cx).form.language.clone());
    for (language, placeholder) in [
        (Language::English, "Automatic"),
        (Language::Chinese, "自动"),
    ] {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            select.update(cx, |select, cx| {
                select.set_selected_value(&language, window, cx);
                cx.emit(SelectEvent::Confirm(Some(language)));
            });
        })
        .expect("select interface language");
        cx.run_until_parked();
        assert_eq!(read(&fixture).language, language);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let form = &fixture.view.read(cx).form;
            assert_eq!(crate::i18n::language(), language);
            assert_eq!(form.language.entity_id(), select.entity_id());
            assert_eq!(
                form.fallbacks
                    .read(cx)
                    .presentation()
                    .placeholder()
                    .as_ref(),
                placeholder
            );
        })
        .expect("language applies without replacing the controls");
    }
}

#[gpui_kit::test]
fn font_controls_step_from_effective_values_and_reset_only_their_overrides(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(
        cx,
        Preferences {
            mono_font_family: Some("Fixture Mono".into()),
            ..builtin()
        },
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |page, cx| {
            page.scroll_to = Some(Group::Fonts.index());
            cx.notify();
        });
        window.render_frame(cx);
        let initial = f32::from(cx.theme().font_size);
        assert!(window.try_find("appearance-ui-font-default").is_none());
        assert!(window.try_find("appearance-ui-size-row").is_none());
        assert_eq!(
            fixture
                .view
                .read(cx)
                .form
                .ui_size
                .read(cx)
                .presentation()
                .placeholder()
                .as_ref(),
            initial.to_string()
        );
        window.click("appearance-ui-size-increase", cx);
        assert_eq!(read(&fixture).ui_font_size, Some(initial + 1.));
        window.render_frame(cx);
        window.click("appearance-ui-size-decrease", cx);
        assert_eq!(read(&fixture).ui_font_size, Some(initial));
        window.click("appearance-ui-font-default", cx);
        assert_eq!(read(&fixture).ui_font_size, None);
        assert_eq!(
            read(&fixture).mono_font_family.as_deref(),
            Some("Fixture Mono")
        );
        window.render_frame(cx);
        assert!(window.try_find("appearance-ui-font-default").is_none());
        window.click("appearance-mono-font-default", cx);
        assert_eq!(read(&fixture).mono_font_family, None);
        let content = f32::from(theme::content_font_size(cx));
        window.click("appearance-content-size-increase", cx);
        assert_eq!(read(&fixture).content_font_size, Some(content + 1.));
        window.render_frame(cx);
        window.click("appearance-content-size-default", cx);
        assert_eq!(read(&fixture).content_font_size, None);
        assert_eq!(read(&fixture).theme_source, ThemeSource::Builtin);
    })
    .expect("font controls persist and inherit");
}

#[gpui_kit::test]
fn font_steppers_preserve_invalid_edits_and_fit_narrow_settings(cx: &mut TestAppContext) {
    let fixture = fixture(cx, builtin());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |page, cx| {
            page.scroll_to = Some(Group::Fonts.index());
            page.form
                .content_size
                .update(cx, |input, cx| input.set_value("invalid", window, cx));
            cx.notify();
        });
        window.resize(size(px(480.), px(720.)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.click("appearance-content-size-increase", cx);
        assert_eq!(
            fixture.view.read(cx).form.content_size.read(cx).value(),
            "invalid"
        );
        assert_eq!(read(&fixture).content_font_size, None);
        for id in [
            "appearance-ui-size-stepper",
            "appearance-content-size-stepper",
        ] {
            let bounds = window.find(id).bounds();
            assert!(bounds.left() >= px(0.) && bounds.right() <= px(480.));
        }
        window.click("appearance-content-size-default", cx);
        assert!(
            fixture
                .view
                .read(cx)
                .form
                .content_size
                .read(cx)
                .value()
                .is_empty()
        );
    })
    .expect("safe font stepper boundaries");
}

#[gpui_kit::test]
fn layout_numbers_share_stepping_inheritance_and_boundary_rules(cx: &mut TestAppContext) {
    let fixture = fixture(cx, builtin());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |page, cx| {
            page.scroll_to = Some(Group::Layout.index());
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("appearance-line-height-default").is_none());
        assert!(window.try_find("appearance-opacity-default").is_none());
        assert_eq!(
            fixture
                .view
                .read(cx)
                .form
                .line_height
                .read(cx)
                .presentation()
                .placeholder()
                .as_ref(),
            "1.6"
        );
        window.click("appearance-line-height-increase", cx);
        assert_eq!(read(&fixture).content_line_height, Some(1.7));
        window.click("appearance-line-height-decrease", cx);
        assert_eq!(read(&fixture).content_line_height, Some(1.6));
        window.click("appearance-line-height-default", cx);
        assert_eq!(read(&fixture).content_line_height, None);
        for (value, action, expected) in [
            ("0.98", "appearance-opacity-increase", 1.),
            ("1", "appearance-opacity-increase", 1.),
            ("0.02", "appearance-opacity-decrease", 0.),
            ("0", "appearance-opacity-decrease", 0.),
        ] {
            fixture.view.update(cx, |page, cx| {
                page.form
                    .opacity
                    .update(cx, |input, cx| input.set_value(value, window, cx));
                page.flush_pending(window, cx);
            });
            window.render_frame(cx);
            window.click(action, cx);
            assert_eq!(read(&fixture).content_opacity, Some(expected));
        }
        window.click("appearance-opacity-default", cx);
        window.render_frame(cx);
        assert_eq!(read(&fixture).content_opacity, None);
        assert!(window.try_find("appearance-opacity-default").is_none());
    })
    .expect("numeric controls in layout settings");
}

#[gpui_kit::test]
fn text_overrides_have_contextual_reset_icons_without_changing_other_fields(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(
        cx,
        Preferences {
            font_fallbacks: Some(vec![]),
            content_line_height: Some(1.8),
            ..builtin()
        },
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |page, cx| {
            page.scroll_to = Some(Group::Fonts.index());
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("appearance-theme-file-default").is_none());
        let reset = window.find("appearance-fallbacks-default").bounds();
        let input = window.find("appearance-fallbacks").bounds();
        assert_eq!(reset.size.width, px(crate::ui::COMPACT_CONTROL));
        assert!(reset.right() <= input.left());
        assert_eq!(fixture.view.read(cx).form.fallbacks.read(cx).value(), "[]");
        window.click("appearance-fallbacks-default", cx);
        window.render_frame(cx);
        assert_eq!(read(&fixture).font_fallbacks, None);
        assert_eq!(read(&fixture).content_line_height, Some(1.8));
        assert!(window.try_find("appearance-fallbacks-default").is_none());
    })
    .expect("text override reset");
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
            assert!(page.message.contains("请检查设置"));
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
        window.input(crate::i18n::tr!("appearance-font-default"), cx);
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
    cx.update(|cx| {
        crate::configuration::init(Some(fixture.path.clone()), cx);
        theme::init(None, cx);
    });
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
    cx.update(|cx| {
        crate::configuration::init(Some(fixture.path.clone()), cx);
        theme::init(None, cx);
    });
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        fixture.view.update(cx, |page, cx| {
            assert_eq!(page.form.ui_size.read(cx).value(), "24");
            page.flush_pending(window, cx);
            assert!(page.message.contains("其他位置修改"));
        });
        assert_eq!(Theme::global(cx).font_size, px(27.));
    })
    .expect("conflict preserves local input and external configuration");
    assert_eq!(read(&fixture).ui_font_size, Some(27.));
}

#[gpui_kit::test]
fn all_fields_round_trip_and_default_is_applied_immediately(cx: &mut TestAppContext) {
    let expected = Preferences {
        language: crate::i18n::Language::Chinese,
        ui_font_family: Some("UI custom".into()),
        mono_font_family: Some("Mono custom".into()),
        font_fallbacks: Some(vec!["Fallback A".into()]),
        ui_font_size: Some(17.),
        content_font_size: Some(19.),
        content_line_height: Some(1.8),
        content_opacity: Some(0.8),
        composer_enter_behavior: ComposerEnterBehavior::Newline,
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

#[gpui_kit::test]
fn message_enter_setting_persists_and_reopens_with_the_saved_choice(cx: &mut TestAppContext) {
    let fixture = fixture(cx, builtin());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |page, cx| {
            page.query = "Enter".into();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("appearance-composer-enter-row").visible());
        let select = fixture.view.read(cx).form.composer_enter.clone();
        select.update(cx, |select, cx| {
            select.set_selected_value(&ComposerEnterBehavior::Newline, window, cx);
            cx.emit(SelectEvent::Confirm(Some(ComposerEnterBehavior::Newline)));
        });
    })
    .expect("change Enter setting");
    assert_eq!(
        read(&fixture).composer_enter_behavior,
        ComposerEnterBehavior::Newline
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let reopened =
            cx.new(|cx| AppearancePage::new(theme::Editor::at(fixture.path.clone()), window, cx));
        assert_eq!(
            reopened
                .read(cx)
                .form
                .composer_enter
                .read(cx)
                .selected_value(),
            Some(&ComposerEnterBehavior::Newline)
        );
        assert_eq!(
            cx.global::<theme::Appearance>().composer_enter_behavior,
            ComposerEnterBehavior::Newline
        );
    })
    .expect("reopen saved Enter setting");
}
