use super::*;

#[test]
fn upstream_text_style_preserves_component_theme() {
    let mut app = gpui_kit::TestApp::new();
    app.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
        let style = markdown_style(cx);
        assert_eq!(style.foreground(), cx.theme().foreground);
        assert_eq!(style.selection(), cx.theme().selection);
        assert_eq!(style.link(), cx.theme().link);
        assert_eq!(style.border(), cx.theme().border);
        assert_eq!(style.code_background(), cx.theme().muted);
        assert_eq!(style.is_dark(), cx.theme().is_dark());
        assert_eq!(
            style.table_head().background,
            Some(cx.theme().table_head.into()),
        );
        let radius = cx.theme().semantic_tokens().radius.md;
        assert_eq!(style.table().corner_radii.top_left, Some(radius.into()));
        assert!(gpui_kit::base::TextViewDefaults::global(cx).has_code_block_highlighter());
    });
}
