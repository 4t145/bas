use super::*;
use crate::theme::source::Files;

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
