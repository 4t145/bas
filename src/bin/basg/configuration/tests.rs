use super::*;

#[gpui_kit::test]
fn selected_store_is_available_without_theme_and_survives_theme_initialization(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().expect("isolated configuration");
    let path = directory.path().join("selected.json");
    let theme_path = directory.path().join("theme.json");
    cx.update(|cx| {
        init(Some(path.clone()), cx);
        let selected = store(cx);
        assert_eq!(selected.path(), path);
        let mut document = selected.load().expect("default configuration");
        let appearance = bas::config::Appearance {
            ui_font_size: Some(17.0),
            ..Default::default()
        };
        selected
            .save_appearance(&mut document, appearance.clone())
            .expect("configuration writes need no theme or settings page");

        gpui_kit::init(cx);
        crate::theme::init(Some(theme_path.clone()), cx);
        assert_eq!(store(cx).path(), path);
        assert_eq!(
            store(cx).load().expect("same store").settings.appearance,
            appearance
        );
        let editor = crate::theme::Editor::current(cx);
        assert_eq!(editor.appearance_path(), path);
        assert_eq!(editor.theme_override(), Some(theme_path.as_path()));
        assert_eq!(
            editor
                .load()
                .expect("appearance from selected store")
                .preferences,
            document.settings.appearance
        );
    });
    assert!(
        !theme_path.exists(),
        "selecting a theme never creates another settings store"
    );
}
