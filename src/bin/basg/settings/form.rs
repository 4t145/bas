//! Shared geometry for settings fields, independent of their persistence.

use crate::ui;
use gds_uim::el;
use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    AnyElement, App, IntoElement, RenderOnce, TestSupportExt, Window, div, prelude::*, px,
};

pub(super) const CONTROLS_WIDTH: f32 = 320.;

#[derive(IntoElement)]
pub(super) struct Row {
    pub id: String,
    pub label: &'static str,
    pub help: &'static str,
    pub control: AnyElement,
    pub reset: Option<AnyElement>,
    pub last: bool,
}

impl RenderOnce for Row {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        el! { .div {
            # format!("{}-row", self.id); test_support;
            flex; items_center; gap_3; py_3; flex_shrink_0;
            @if (!self.last)
                border_b_1; border_color: $cx.theme().border;
            @end;
        } [
            .div { flex_1; min_w_0; flex; flex_col; gap_1; } [
                .div { # format!("{}-label", self.id); test_support; } [$self.label],
                .div { text_color: $cx.theme().muted_foreground; text_size: ui::SMALL; } [$self.help],
            ],
            .div {
                # format!("{}-controls", self.id); test_support;
                flex; items_center; justify_end; gap_2; w: CONTROLS_WIDTH.px; flex_shrink_0;
            } [
                .div { w: ui::COMPACT_CONTROL.px; flex_shrink_0; } [..$self.reset],
                $self.control,
            ],
        ] }
    }
}
