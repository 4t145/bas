use super::*;

fn family() -> String {
    json!({"themes": [
        {"name": "DankShell Dark", "appearance": "dark", "style": {
            "background.appearance": "opaque", "panel.background": "#1a110f",
            "editor.background": "#1a110f", "text": "#f1dfd9"
        }},
        {"name": "DankShell Dark Transparent", "appearance": "dark", "style": {
            "background.appearance": "blurred", "panel.background": "#1a110fB6",
            "editor.background": "#1a110fB6", "text": "#f1dfd9"
        }},
        {"name": "DankShell Light Transparent", "appearance": "light", "style": {
            "background.appearance": "transparent", "panel.background": "#fff8f6B6",
            "editor.background": "#fff8f6B6", "text": "#271d1a"
        }}
    ]})
    .to_string()
}

#[test]
fn follows_jsonc_selection_system_appearance_and_preserves_alpha() {
    let files = Files { zed_theme: Some(family()),
        zed_settings: Some(r#"{
            // Zed settings commonly include comments and trailing commas.
            "theme": {"mode":"system","dark":"DankShell Dark Transparent","light":"DankShell Light Transparent",},
            "ui_font_size": 16,
            "ui_font_fallbacks": ["Sarasa UI SC", "Noto Sans CJK JP"],
        }"#.into()), ..Files::default() };
    let dark = resolve(&files, true).unwrap();
    assert_eq!(dark.name, "DankShell Dark Transparent");
    assert!(dark.config.mode.is_dark());
    assert_eq!(dark.config.colors.sidebar.as_deref(), Some("#1a110fB6"));
    assert_eq!(dark.config.colors.background.as_deref(), Some("#1a110fB6"));
    assert_eq!(
        dark.config.colors.primary_foreground.as_deref(),
        Some("#1a110f")
    );
    assert!(matches!(
        dark.background,
        WindowBackgroundAppearance::Blurred
    ));
    assert_eq!(dark.config.font_size, Some(16.));
    assert_eq!(dark.font_fallbacks, ["Sarasa UI SC", "Noto Sans CJK JP"]);
    let light = resolve(&files, false).unwrap();
    assert!(!light.config.mode.is_dark());
    assert_eq!(light.name, "DankShell Light Transparent");
    assert!(matches!(
        light.background,
        WindowBackgroundAppearance::Transparent
    ));
}

#[test]
fn explicit_variant_and_overrides_take_precedence() {
    let files = Files {
        zed_theme: Some(family()),
        zed_settings: Some(
            json!({"theme":"DankShell Dark",
            "theme_overrides":{"panel.background":"#11223380"}})
            .to_string(),
        ),
        ..Files::default()
    };
    let theme = resolve(&files, false).unwrap();
    assert!(theme.config.mode.is_dark());
    assert_eq!(theme.config.colors.sidebar.as_deref(), Some("#11223380"));
    assert!(matches!(
        theme.background,
        WindowBackgroundAppearance::Opaque
    ));
}

#[test]
fn partial_writes_are_rejected_and_absent_files_use_system_palette() {
    let files = Files {
        zed_theme: Some("{".into()),
        ..Files::default()
    };
    assert!(resolve(&files, true).is_err());
    assert!(
        resolve(&Files::default(), true)
            .unwrap()
            .config
            .mode
            .is_dark()
    );
    assert!(
        !resolve(&Files::default(), false)
            .unwrap()
            .config
            .mode
            .is_dark()
    );
}

#[test]
fn dms_palette_works_without_zed_and_uses_desktop_fonts() {
    let files = Files {
        dms_colors: Some(
            json!({"colors":{"dark":{
                "surface":"#121212", "primary":"#ffb59c", "on_surface":"#eeeeee"
            }}})
            .to_string(),
        ),
        dms_settings: Some(
            json!({
                "fontFamily":"Inter", "monoFontFamily":"Fira Code"
            })
            .to_string(),
        ),
        ..Files::default()
    };
    let theme = resolve(&files, true).unwrap();
    assert_eq!(theme.name, "DMS");
    assert_eq!(theme.config.colors.background.as_deref(), Some("#121212"));
    assert_eq!(theme.config.font_family.as_deref(), Some("Inter"));
}
