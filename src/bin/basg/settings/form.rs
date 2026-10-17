//! Shared geometry for settings fields, independent of their persistence.

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
        gds_uim::el! { #div {
            id: $format!("{}-row", self.id); test_support;
            flex; items_center; gap_3; py_3; flex_shrink_0;
            @$!self.last => { border_b_1; border_color: $cx.theme().border; };
        } [
            #div { flex_1; min_w_0; flex; flex_col; gap_1; } [
                #div { id: $format!("{}-label", self.id); test_support; } [$self.label],
                #div { text_color: $cx.theme().muted_foreground; text_size: crate::ui::SMALL; } [$self.help],
            ],
            #div {
                id: $format!("{}-controls", self.id); test_support;
                flex; items_center; justify_end; gap_2; w: $px(CONTROLS_WIDTH); flex_shrink_0;
            } [
                #div { w: $px(crate::ui::COMPACT_CONTROL); flex_shrink_0; } [..$self.reset],
                $self.control,
            ],
        ] }
    }
}
