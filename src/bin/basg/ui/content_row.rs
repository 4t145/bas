//! Passive content-row layout. Only the controls supplied by callers are interactive.

use crate::ui;
use gds_uim::el;
use gpui_kit::base::StyledExt;
use gpui_kit::{
    AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, StyleRefinement, Styled,
    TestSupportExt, Window,
    assets::IconName,
    component::{ActiveTheme, Icon, Sizable},
    div,
    prelude::*,
    px,
};

#[derive(IntoElement)]
pub(crate) struct ContentRow {
    id: ElementId,
    icon: Option<IconName>,
    content: AnyElement,
    actions: Vec<AnyElement>,
    style: StyleRefinement,
}

impl ContentRow {
    pub(crate) fn new(id: impl Into<ElementId>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            icon: None,
            content: content.into_any_element(),
            actions: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub(crate) fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

impl ParentElement for ContentRow {
    fn extend(&mut self, actions: impl IntoIterator<Item = AnyElement>) {
        self.actions.extend(actions);
    }
}

impl Styled for ContentRow {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ContentRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        el! {
            .div {
                # $self.id; test_support; w_full; min_w_0;
                flex; items_center; gap_2; refine_style: $&self.style;
            } [
                @if (let Some(icon) = self.icon)
                    icon.Icon::new { with_size: ui::ICON.px; flex_shrink_0; text_color: $cx.theme().muted_foreground; },
                @end,
                .div { # "content-row-body"; test_support; flex_1; min_w_0; } [$self.content],
                .div {
                    # "content-row-actions"; test_support;
                    flex; items_center; gap_1; flex_shrink_0;
                } [..$self.actions],
            ]
        }
    }
}
