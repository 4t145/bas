use super::*;
use gpui_kit::{
    AppContext, Context, Render, TestAppContext,
    component::{
        Root, Theme, ThemeMode,
        input::{Editor, EditorState, Input, InputState},
    },
    div,
    prelude::*,
    px, size,
    test::TestWindowExt,
};

struct Probe {
    input: Entity<InputState>,
    editor: Entity<EditorState>,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .text_color(cx.theme().foreground)
            .children([
                placeholder(&self.input, Input::new(&self.input).id("plain")),
                placeholder(&self.editor, Editor::new(&self.editor).h(px(150.))),
            ])
    }
}

#[gpui_kit::test]
fn placeholder_palette_follows_secondary_icons_without_dimming_input(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut view = None;
    let window = cx.open_window(size(px(500.), px(300.)), |window, cx| {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search…"));
        let editor = cx.new(|cx| EditorState::new(window, cx).placeholder("Message…"));
        let probe = cx.new(|_| Probe { input, editor });
        view = Some(probe.clone());
        Root::new(probe, window, cx)
    });
    let view = view.expect("input fixture");
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update_window(window.into(), |_, window, cx| {
            Theme::change(mode, Some(window), cx);
            window.render_frame(cx);
            let style = placeholder_style(cx);
            assert_eq!(style.muted_foreground, crate::ui::secondary_foreground(cx));
            assert!(style.muted_foreground.a < cx.theme().muted_foreground.a);
            assert_eq!(style.foreground, cx.theme().foreground);
            assert_eq!(style.caret, cx.theme().caret);
            assert_eq!(style.selection, cx.theme().selection);
            window.click("plain", cx);
            window.input("real input", cx);
            assert_eq!(view.read(cx).input.read(cx).value(), "real input");
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            window.render_frame(cx);
            assert!(view.read(cx).input.read(cx).value().is_empty());
            assert_eq!(window.find("plain").label(), Some("Search…"));
        })
        .expect("native input behavior with shared placeholder colors");
    }
}
