//! Placeholder policy applied after the native component projects its editor palette.

use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window,
    base::input::{InputBaseState, InputEditorStyle, InputModeKind},
    component::ActiveTheme,
};

pub(crate) fn placeholder<M: InputModeKind>(
    state: &Entity<InputBaseState<M>>,
    input: impl IntoElement,
) -> AnyElement {
    Placeholder {
        state: state.clone(),
        input: input.into_any_element(),
    }
    .into_any_element()
}

struct Placeholder<M: InputModeKind> {
    state: Entity<InputBaseState<M>>,
    input: AnyElement,
}

fn placeholder_style(cx: &App) -> InputEditorStyle {
    let theme = cx.theme();
    InputEditorStyle {
        foreground: theme.foreground,
        muted_foreground: super::secondary_foreground(cx),
        background: theme
            .highlight_theme
            .style
            .editor_background
            .unwrap_or(theme.background),
        border: theme.border,
        selection: theme.selection,
        caret: theme.caret,
        ..Default::default()
    }
}

impl<M: InputModeKind> IntoElement for Placeholder<M> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<M: InputModeKind> Element for Placeholder<M> {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let layout = self.input.request_layout(window, cx);
        // Nonempty editors retain the component's complete syntax/folding palette.
        // An empty editor only paints its hint, caret and selection.
        if self.state.read(cx).value().is_empty() {
            let style = placeholder_style(cx);
            self.state
                .update(cx, |state, _| state.set_editor_style(style));
        }
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.input.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.input.paint(window, cx);
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
