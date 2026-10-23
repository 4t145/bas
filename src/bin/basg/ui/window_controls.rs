//! First-party native titlebars share the workspace's compact button scale.

use super::{Button, COMPACT_CONTROL, SMALL, SMALL_ICON};
use crate::i18n::tr;
use std::{cell::Cell, rc::Rc};

use gds_uim::el;
use gpui_kit::{
    AnyElement, App, ClickEvent, Decorations, IntoElement, MouseButton, ParentElement, RenderOnce,
    SharedString, Styled, TestSupportExt, Window, WindowControlArea, WindowOptions,
    assets::IconName,
    component::{ActiveTheme, Icon, InteractiveElementExt, Sizable, button::ButtonVariants},
    div,
    prelude::*,
    px,
};

pub(crate) const HEIGHT: f32 = 44.;
pub(crate) const MACOS_CONTROLS_WIDTH: f32 = 80.;
type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct WindowControls {
    maximize: Option<Handler>,
    close: Option<Handler>,
}

impl WindowControls {
    pub(crate) fn new() -> Self {
        Self {
            maximize: None,
            close: None,
        }
    }

    pub(crate) fn on_maximize(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.maximize = Some(Box::new(handler));
        self
    }

    pub(crate) fn on_close_window(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.close = Some(Box::new(handler));
        self
    }
}

fn control(
    id: &'static str,
    icon: IconName,
    hint: SharedString,
    area: WindowControlArea,
) -> Button {
    el! {
        id.Button::new {
            ghost; xsmall; w: COMPACT_CONTROL.px; h: COMPACT_CONTROL.px;
            p_0; flex_shrink_0;
        } [
            .div {
                # format!("{id}-icon").SharedString::from; test_support; flex;
            } [icon.Icon::new { with_size: SMALL_ICON.px; }],
        ] {
            accessibility_label: ~hint; tooltip: hint;
            when: cfg!(target_os = "windows"), { window_control_area: area };
            on_mouse_down: MouseButton::Left, $|_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            };
        }
    }
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let controls = el! {
            .div { # "window-controls"; flex; items_center; gap_1; flex_shrink_0; }
        };
        if cfg!(target_os = "macos") || cfg!(target_family = "wasm") {
            return controls;
        }
        let supported = window.window_controls();
        let maximize = supported.maximize.then(|| {
            if window.is_maximized() {
                (IconName::WindowRestore, tr!("window-restore"))
            } else {
                (IconName::WindowMaximize, tr!("window-maximize"))
            }
        });
        el! {
            controls [
                @if (supported.minimize)
                    (
                        "window-minimize",
                        IconName::WindowMinimize,
                        tr!("window-minimize") { into },
                        WindowControlArea::Min,
                    ).control {
                        on_click: $|_, window, cx| {
                            cx.stop_propagation();
                            window.minimize_window();
                        };
                    },
                @end,
                @if (let Some((icon, hint)) = maximize)
                    ("window-maximize", icon, hint { into }, WindowControlArea::Max).control {
                        on_click: $move |event, window, cx| {
                            cx.stop_propagation();
                            if let Some(handler) = &self.maximize {
                                handler(event, window, cx);
                            } else {
                                window.zoom_window();
                            }
                        };
                    },
                @end,
                (
                    "window-close",
                    IconName::WindowClose,
                    tr!("window-close") { into },
                    WindowControlArea::Close,
                ).control {
                    on_click: $move |event, window, cx| {
                        cx.stop_propagation();
                        if let Some(handler) = &self.close {
                            handler(event, window, cx);
                        } else {
                            window.remove_window();
                        }
                    };
                },
            ]
        }
    }
}

#[derive(IntoElement)]
pub(crate) struct TitleBar {
    children: Vec<AnyElement>,
    controls: WindowControls,
}

impl TitleBar {
    pub(crate) fn new() -> Self {
        Self {
            children: Vec::new(),
            controls: WindowControls::new(),
        }
    }

    pub(crate) fn window_options() -> WindowOptions {
        gpui_kit::component::TitleBar::window_options()
    }

    pub(crate) fn on_close_window(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.controls = self.controls.on_close_window(handler);
        self
    }
}

impl ParentElement for TitleBar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let dragging = window
            .use_state(cx, |_, _| Rc::new(Cell::new(false)))
            .read(cx)
            .clone();
        let down = dragging.clone();
        let up = dragging.clone();
        let outside = dragging.clone();
        let title = el! {
            .div {
                # "title-bar-content"; flex; items_center; flex_1; h_full;
                text_size: SMALL; window_control_area: WindowControlArea::Drag;
                on_mouse_down: MouseButton::Left, $move |_, _, _| down.set(true);
                on_mouse_up: MouseButton::Left, $move |_, _, _| up.set(false);
                on_mouse_down_out: $move |_, _, _| outside.set(false);
                on_mouse_move: $move |_, window, _| {
                    if dragging.replace(false) {
                        window.start_window_move();
                    }
                };
                on_double_click: $|_, window, _| {
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else if cfg!(target_os = "linux") {
                        window.zoom_window();
                    }
                };
                @if (cfg!(target_os = "linux")
                    && matches!(window.window_decorations(), Decorations::Client { .. }))
                    on_mouse_down: MouseButton::Right, $|event, window, _| {
                        window.show_window_menu(event.position)
                    };
                @end;
            }
        };
        el! {
            .div {
                # "title-bar"; flex; items_center; gap_1; h: HEIGHT.px; flex_shrink_0; px_3;
                when: $cfg!(target_os = "macos") && !window.is_fullscreen(),
                    { pl: MACOS_CONTROLS_WIDTH.px };
                bg: $cx.theme().title_bar; border_b_1; border_color: $cx.theme().title_bar_border;
            } [
                title [..$self.children],
                $self.controls,
            ]
        }
    }
}
