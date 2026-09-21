use super::*;
use crate::theme::source::Files;

#[test]
fn defaults_keep_desktop_fonts_and_frosted_surface() {
    let desktop = source::resolve(
        &Files {
            zed_settings: Some(
                r#"{"ui_font_family":"Test UI","ui_font_size":16,"agent_ui_font_size":17}"#.into(),
            ),
            ..Files::default()
        },
        true,
    )
    .expect("valid desktop");
    let result = resolve(&Preferences::default(), desktop).expect("valid defaults");
    assert_eq!(
        result.desktop.config.font_family.as_deref(),
        Some("Test UI")
    );
    assert_eq!(result.desktop.config.font_size, Some(16.));
    assert_eq!(result.content_font_size, 17.);
    assert_eq!(result.content_line_height, 1.6);
    assert_eq!(result.content_opacity, 0.94);
    assert!(matches!(
        result.desktop.background,
        WindowBackgroundAppearance::Blurred
    ));
    let result = resolve(&Preferences::default(), source::fallback(true)).expect("valid fallback");
    assert_eq!(result.content_font_size, 13.);
}

#[test]
fn explicit_preferences_override_imports_and_round_trip() {
    let preferences = Preferences::from_json(
        r#"{
        "ui_font_family":"UI", "mono_font_family":"Code", "font_fallbacks":[],
        "ui_font_size":20, "content_font_size":19, "content_line_height":1.8,
        "content_opacity":0.8, "background":"opaque", "theme_mode":"dark"
    }"#,
    )
    .expect("valid preferences");
    let json = serde_json::to_string(&preferences).expect("serializable preferences");
    assert_eq!(
        Preferences::from_json(&json).expect("round trip"),
        preferences
    );
    let result = resolve(&preferences, source::fallback(false)).expect("valid override");
    assert_eq!(result.desktop.config.font_size, Some(20.));
    assert_eq!(result.desktop.config.font_family.as_deref(), Some("UI"));
    assert_eq!(
        result.desktop.config.mono_font_family.as_deref(),
        Some("Code")
    );
    assert!(result.desktop.font_fallbacks.is_empty());
    assert_eq!(result.content_font_size, 19.);
    assert_eq!(result.content_line_height, 1.8);
    assert_eq!(result.content_opacity, 0.8);
    assert!(matches!(
        result.desktop.background,
        WindowBackgroundAppearance::Opaque
    ));
}

#[test]
fn sizes_and_opacity_are_validated_for_json_and_programmatic_callers() {
    for name in ["ui_font_size", "content_font_size", "content_line_height"] {
        for value in ["0", "-1", "1e100", "\"large\""] {
            assert!(Preferences::from_json(&format!(r#"{{"{name}":{value}}}"#)).is_err());
        }
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0., -1.] {
        assert!(
            Preferences {
                ui_font_size: Some(value),
                ..Preferences::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Preferences {
                content_font_size: Some(value),
                ..Preferences::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Preferences {
                content_line_height: Some(value),
                ..Preferences::default()
            }
            .validate()
            .is_err()
        );
    }
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(
            Preferences {
                content_opacity: Some(value),
                ..Preferences::default()
            }
            .validate()
            .is_err()
        );
    }
    for value in [0., 1.] {
        assert!(
            Preferences {
                content_opacity: Some(value),
                ..Preferences::default()
            }
            .validate()
            .is_ok()
        );
    }
    assert!(Preferences::from_json(r#"{"ui_font_sze":17}"#).is_err());
    assert!(Preferences::from_json(r#"{"background":"glass"}"#).is_err());
}

#[test]
fn all_imported_font_size_entries_are_validated() {
    for name in ["ui_font_size", "agent_ui_font_size"] {
        for value in ["0", "-1", "1e100", "\"large\"", "null"] {
            let files = Files {
                zed_settings: Some(format!(r#"{{"{name}":{value}}}"#)),
                ..Files::default()
            };
            assert!(source::resolve(&files, true).is_err());
        }
    }
}

#[test]
fn background_auto_preserves_import_and_other_policies_are_explicit() {
    for (policy, expected) in [
        (Background::Auto, WindowBackgroundAppearance::Opaque),
        (Background::Opaque, WindowBackgroundAppearance::Opaque),
        (
            Background::Transparent,
            WindowBackgroundAppearance::Transparent,
        ),
        (Background::Blurred, WindowBackgroundAppearance::Blurred),
    ] {
        let result = resolve(
            &Preferences {
                background: policy,
                ..Preferences::default()
            },
            source::fallback(true),
        )
        .expect("valid policy");
        assert_eq!(result.desktop.background, expected);
    }
}
