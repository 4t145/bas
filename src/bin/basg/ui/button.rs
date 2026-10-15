//! BasG cursor policy; component buttons retain ownership of all interaction and visuals.

use gpui_kit::{
    AnyElement, App, ClickEvent, CursorStyle, ElementId, InteractiveElement, Interactivity,
    IntoElement, ParentElement, RenderOnce, SharedString, StyleRefinement, Styled, Window,
    component::{
        Disableable, Icon, RoleOverride, Selectable, Sizable, Size,
        button::{Button as ComponentButton, ButtonVariant, ButtonVariants},
    },
};

#[derive(IntoElement)]
pub(crate) struct Button {
    inner: ComponentButton,
    disabled: bool,
    loading: bool,
}

impl Button {
    pub(crate) fn new(id: impl Into<ElementId>) -> Self {
        Self {
            inner: ComponentButton::new(id).line_height(gpui_kit::relative(1.)),
            disabled: false,
            loading: false,
        }
    }

    pub(crate) fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.inner = self.inner.label(label);
        self
    }

    pub(crate) fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.inner = self.inner.icon(icon.into());
        self
    }

    pub(crate) fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.inner = self.inner.accessibility_label(label);
        self
    }

    pub(crate) fn role(mut self, role: impl Into<RoleOverride>) -> Self {
        self.inner = self.inner.role(role);
        self
    }

    pub(crate) fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.inner = self.inner.tooltip(tooltip);
        self
    }

    pub(crate) fn hoverable_tooltip(
        mut self,
        tooltip: impl Fn(&mut Window, &mut App) -> gpui_kit::AnyView + 'static,
    ) -> Self {
        self.inner.interactivity().hoverable_tooltip(tooltip);
        self
    }

    pub(crate) fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.inner = self.inner.tab_stop(tab_stop);
        self
    }

    pub(crate) fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.inner = self.inner.on_click(handler);
        self
    }

    pub(crate) fn into_component(mut self) -> ComponentButton {
        // Explicit styles (e.g. passive status icons) win only while enabled.
        let cursor = if self.disabled || self.loading {
            CursorStyle::Arrow
        } else {
            self.inner
                .style()
                .mouse_cursor
                .unwrap_or(CursorStyle::PointingHand)
        };
        self.inner
            .disabled(self.disabled)
            .loading(self.loading)
            .cursor(cursor)
    }
}

impl Disableable for Button {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Selectable for Button {
    fn selected(mut self, selected: bool) -> Self {
        self.inner = self.inner.selected(selected);
        self
    }

    fn is_selected(&self) -> bool {
        self.inner.is_selected()
    }
}

impl Sizable for Button {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.inner = self.inner.with_size(size);
        self
    }
}

impl ButtonVariants for Button {
    fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.inner = self.inner.with_variant(variant);
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.inner.extend(elements);
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}

impl gpui_kit::component::menu::DropdownMenu for Button {}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_component().render(window, cx)
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
