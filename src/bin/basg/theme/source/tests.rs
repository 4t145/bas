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
