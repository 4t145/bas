//! Shared interface primitives. UI text uses the workspace's compact scale; message and code text
//! use the separate content font preferences.

mod button;
mod content_row;
pub(crate) mod input;
pub(crate) mod input_geometry;
pub(crate) mod session_status;
pub(crate) mod shortcut;
pub(crate) mod time;
pub(crate) mod tooltip;
pub(crate) mod window_controls;
pub(crate) use button::Button;
pub(crate) use content_row::ContentRow;

use gds_uim::el;
use gpui_kit::{
    App, Rems,
    assets::IconName,
    component::{ActiveTheme, Sizable, button::*, menu::PopupMenu},
    prelude::*,
    px,
};

pub(crate) const SMALL: Rems = Rems(0.875);
pub(crate) const HEADING: Rems = Rems(1.125);
pub(crate) const TITLE: Rems = Rems(1.25);
pub(crate) const ICON: f32 = 16.;
pub(crate) const SMALL_ICON: f32 = 12.;
pub(crate) const COMPACT_CONTROL: f32 = 24.;
pub(crate) const CONTROL: f32 = 32.;
pub(crate) const MENU_WIDTH: f32 = 320.;
pub(crate) const MENU_MAX_HEIGHT: f32 = 360.;
pub(crate) const MENU_ROW_HEIGHT: f32 = 28.;
const SUBTLE_FOREGROUND_OPACITY: f32 = 0.5;
const SUBTLE_HOVER_OPACITY: f32 = 0.35;
const SUBTLE_ACTIVE_OPACITY: f32 = 0.55;

pub(crate) fn checkbox(
    id: impl Into<gpui_kit::ElementId>,
) -> gpui_kit::component::checkbox::Checkbox {
    gpui_kit::component::checkbox::Checkbox::new(id).text_size(SMALL)
}

pub(crate) fn icon_button(id: impl Into<gpui_kit::ElementId>, icon: IconName) -> Button {
    el! {
        Button(id) {
            ghost;
            xsmall;
            icon: icon;
            w: COMPACT_CONTROL.px;
            h: COMPACT_CONTROL.px;
            p_0;
            flex_shrink_0;
        }
    }
}

pub(crate) fn action_button(id: impl Into<gpui_kit::ElementId>) -> Button {
    el! {
        Button(id) {
            with_variant: ButtonVariant::Default;
            small;
            h: CONTROL.px;
        }
    }
}

pub(crate) fn icon_action(
    id: impl Into<gpui_kit::ElementId>,
    icon: IconName,
    hint: impl Into<gpui_kit::SharedString>,
) -> Button {
    let hint = hint.into();
    icon_button(id, icon)
        .accessibility_label(hint.clone())
        .tooltip(hint)
}

/// Reading and editing utilities stay quiet until the user targets them.
pub(crate) fn subtle_icon_button(
    id: impl Into<gpui_kit::ElementId>,
    icon: IconName,
    cx: &App,
) -> Button {
    let variant = ButtonCustomVariant::new(cx)
        .foreground(cx.theme().foreground)
        .hover(cx.theme().muted.opacity(SUBTLE_HOVER_OPACITY))
        .active(cx.theme().muted.opacity(SUBTLE_ACTIVE_OPACITY));
    el! {
        #icon_button(id, icon) {
            custom: variant;
            text_color: $secondary_foreground(cx);
        }
    }
}

pub(crate) fn secondary_foreground(cx: &App) -> gpui_kit::Hsla {
    cx.theme()
        .muted_foreground
        .opacity(SUBTLE_FOREGROUND_OPACITY)
}

pub(crate) fn menu(menu: PopupMenu) -> PopupMenu {
    menu.min_w(px(MENU_WIDTH))
        .max_w(px(MENU_WIDTH))
        .max_h(px(MENU_MAX_HEIGHT))
        .scrollable(true)
}
