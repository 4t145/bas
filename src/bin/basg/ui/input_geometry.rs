//! Alignment for controls that accompany a GPUI editor, measured through its public API.

use gpui_kit::{
    App, Entity, EntityInputHandler, Pixels, Window, component::input::EditorState, px,
};

pub(crate) fn content_left(
    editor: &Entity<EditorState>,
    padding: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> Option<Pixels> {
    editor.update(cx, |editor, cx| {
        let text = editor.text_bounds()?;
        let origin = editor.bounds_for_range(0..0, text, window, cx)?;
        Some(padding + (origin.left() - text.left()).max(px(0.)))
    })
}
