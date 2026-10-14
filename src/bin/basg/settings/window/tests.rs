use super::*;

#[gpui_kit::test]
fn settings_request_a_floating_window_with_shared_application_identity(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().expect("isolated settings");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, r#"{"appearance":{"theme_source":"builtin"}}"#)
        .expect("settings fixture");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path), cx);
        theme::init(None, cx);
        let options = window_options(cx);
        assert_eq!(options.kind, WindowKind::Floating);
        assert_eq!(options.app_id.as_deref(), Some("org.bas.Gui"));
        assert_eq!(
            options.window_min_size,
            Some(size(px(MIN_WIDTH), px(MIN_HEIGHT)))
        );
        assert!(options.is_resizable);
        assert!(matches!(
            options.window_bounds,
            Some(WindowBounds::Windowed(bounds)) if bounds.size == size(px(WIDTH), px(HEIGHT))
        ));
    });
}
