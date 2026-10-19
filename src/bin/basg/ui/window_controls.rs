//! First-party native titlebars share the workspace's compact button scale.

use std::{cell::Cell, rc::Rc};

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
) -> super::Button {
    super::Button::new(id)
        .ghost()
        .xsmall()
        .w(px(super::COMPACT_CONTROL))
        .h(px(super::COMPACT_CONTROL))
        .p_0()
        .flex_shrink_0()
        .child(
            div()
                .id(gpui_kit::SharedString::from(format!("{id}-icon")))
                .test_support()
                .flex()
                .child(Icon::new(icon).with_size(px(super::SMALL_ICON))),
        )
        .accessibility_label(hint.clone())
        .tooltip(hint)
        .when(cfg!(target_os = "windows"), |button| {
            button.window_control_area(area)
        })
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let mut controls = div()
            .id("window-controls")
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0();
        if cfg!(target_os = "macos") || cfg!(target_family = "wasm") {
            return controls;
        }
        let supported = window.window_controls();
        if supported.minimize {
            controls = controls.child(
                control(
                    "window-minimize",
                    IconName::WindowMinimize,
                    crate::i18n::tr!("window-minimize").into(),
                    WindowControlArea::Min,
                )
                .on_click(|_, window, cx| {
                    cx.stop_propagation();
                    window.minimize_window();
                }),
            );
        }
        if supported.maximize {
            let (icon, hint) = if window.is_maximized() {
                (IconName::WindowRestore, crate::i18n::tr!("window-restore"))
            } else {
                (
                    IconName::WindowMaximize,
                    crate::i18n::tr!("window-maximize"),
                )
            };
            controls = controls.child(
                control("window-maximize", icon, hint.into(), WindowControlArea::Max).on_click(
                    move |event, window, cx| {
                        cx.stop_propagation();
                        if let Some(handler) = &self.maximize {
                            handler(event, window, cx);
                        } else {
                            window.zoom_window();
                        }
                    },
                ),
            );
        }
        controls.child(
            control(
                "window-close",
                IconName::WindowClose,
                crate::i18n::tr!("window-close").into(),
                WindowControlArea::Close,
            )
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                if let Some(handler) = &self.close {
                    handler(event, window, cx);
                } else {
                    window.remove_window();
                }
            }),
        )
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
        let mut title = div()
            .id("title-bar-content")
            .flex()
            .items_center()
            .flex_1()
            .h_full()
            .text_size(super::SMALL)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, move |_, _, _| down.set(true))
            .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
            .on_mouse_down_out(move |_, _, _| outside.set(false))
            .on_mouse_move(move |_, window, _| {
                if dragging.replace(false) {
                    window.start_window_move();
                }
            })
            .on_double_click(|_, window, _| {
                if cfg!(target_os = "macos") {
                    window.titlebar_double_click();
                } else if cfg!(target_os = "linux") {
                    window.zoom_window();
                }
            });
        if cfg!(target_os = "linux")
            && matches!(window.window_decorations(), Decorations::Client { .. })
        {
            title = title.on_mouse_down(MouseButton::Right, |event, window, _| {
                window.show_window_menu(event.position)
            });
        }
        div()
            .id("title-bar")
            .flex()
            .items_center()
            .gap_1()
            .h(px(HEIGHT))
            .flex_shrink_0()
            .px_3()
            .when(
                cfg!(target_os = "macos") && !window.is_fullscreen(),
                |bar| bar.pl(px(MACOS_CONTROLS_WIDTH)),
            )
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .child(title.children(self.children))
            .child(self.controls)
    }
}
