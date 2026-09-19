//! Keep compact-window menus on a separate surface, using the component menu itself.

use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    AnyWindowHandle, Bounds, DismissEvent, FocusHandle, Focusable, MouseButton,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, canvas,
    component::menu::{DropdownMenu, PopupMenu},
    point,
    popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions},
};

use super::*;

const MENU_WIDTH: f32 = 320.;
const MENU_MAX_HEIGHT: f32 = 360.;
const MENU_ROW_HEIGHT: f32 = 28.;
const MENU_PADDING: f32 = 12.;

type Builder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

pub(super) struct OpenMenu {
    pub window: Option<AnyWindowHandle>,
    pub content: Entity<PopupMenu>,
    pub size: Size<Pixels>,
    trigger: Bounds<Pixels>,
    previous_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub(super) fn menu_button(
        &self,
        button: Button,
        rows: usize,
        disabled: bool,
        build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.compact {
            return button.dropdown_menu(build).into_any_element();
        }
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        let keyboard_bounds = bounds.clone();
        let builder: Builder = Rc::new(build);
        let keyboard_builder = builder.clone();
        let trigger = button
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if !disabled {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.open_menu(bounds.get(), rows, builder.clone(), window, cx);
                    }
                }),
            )
            .on_key_down(
                cx.listener(move |this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if !disabled
                        && matches!(event.keystroke.key.as_str(), "enter" | "space" | "down")
                    {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.open_menu(
                            keyboard_bounds.get(),
                            rows,
                            keyboard_builder.clone(),
                            window,
                            cx,
                        );
                    }
                }),
            );
        div()
            .relative()
            .flex()
            .flex_shrink_0()
            .child(trigger)
            .child(
                canvas(move |bounds, _, _| measured.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .into_any_element()
    }

    fn open_menu(
        &mut self,
        trigger: Bounds<Pixels>,
        rows: usize,
        build: Builder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_reply_preview(window, cx);
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.trigger == trigger)
        {
            self.close_menu(window, cx);
            return;
        }
        self.close_menu(window, cx);
        let screen = window
            .display(cx)
            .map(|display| display.bounds().size)
            .unwrap_or(size(px(MENU_WIDTH), px(MENU_MAX_HEIGHT * 2.)));
        let menu_size = size(
            px(MENU_WIDTH).min(screen.width * 0.8),
            px(rows as f32 * MENU_ROW_HEIGHT + MENU_PADDING)
                .min(px(MENU_MAX_HEIGHT))
                .min(screen.height * 0.6),
        );
        let previous_focus = window.focused(cx);
        let content = PopupMenu::build(window, cx, |menu, window, cx| {
            build(menu, window, cx)
                .scrollable(true)
                .min_w(menu_size.width - px(2.))
                .max_w(menu_size.width - px(2.))
                .max_h(menu_size.height - px(2.))
        });
        let parent = window.window_handle();
        let menu = content.clone();
        let native =
            match cx.open_window(popup_options(parent, trigger, menu_size), |window, cx| {
                menu.focus_handle(cx).focus(window, cx);
                cx.new(|cx| {
                    Root::new(menu, window, cx)
                        .refine_style(&theme::font_style(cx))
                        .bordered(false)
                        .window_shadow_size(px(0.))
                })
            }) {
                Ok(handle) => Some(handle.into()),
                Err(error) => {
                    tracing::debug!(%error, "using an inline menu on this platform");
                    content.focus_handle(cx).focus(window, cx);
                    None
                }
            };
        let dismissed =
            cx.subscribe_in(&content, window, |this, _, _: &DismissEvent, window, cx| {
                this.close_menu(window, cx);
            });
        let view = cx.entity().downgrade();
        let closed = cx.on_window_closed(move |cx, id| {
            if native.is_some_and(|popup: AnyWindowHandle| popup.window_id() == id) {
                let _ = parent.update(cx, |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.close_menu(window, cx));
                });
            }
        });
        self.menu = Some(OpenMenu {
            window: native,
            content,
            size: menu_size,
            trigger,
            previous_focus,
            _subscriptions: vec![dismissed, closed],
        });
        if native.is_none() {
            self.resize_window(window, cx);
        }
        cx.notify();
    }

    pub(super) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        if let Some(popup) = menu.window {
            if popup
                .update(cx, |_, popup, _| popup.remove_window())
                .is_err()
            {
                // A menu action can still be borrowing its popup window.
                cx.defer(move |cx| {
                    let _ = popup.update(cx, |_, popup, _| popup.remove_window());
                });
            }
        } else {
            self.resize_window(window, cx);
            if menu.content.focus_handle(cx).contains_focused(window, cx)
                && let Some(previous) = menu.previous_focus
            {
                previous.focus(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn close_menu_outside(
        &mut self,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.window.is_some() && !menu.trigger.contains(&position))
        {
            self.close_menu(window, cx);
        }
    }
}

fn popup_options(
    parent: AnyWindowHandle,
    trigger: Bounds<Pixels>,
    menu_size: Size<Pixels>,
) -> WindowOptions {
    WindowOptions {
        kind: WindowKind::AnchoredPopup(PopupOptions {
            parent,
            anchor_rect: trigger,
            anchor: PopupAnchor::TopLeft,
            gravity: PopupGravity::TopRight,
            constraint_adjustment: PopupConstraintAdjustment::FLIP_Y
                | PopupConstraintAdjustment::SLIDE_X
                | PopupConstraintAdjustment::SLIDE_Y,
            offset: point(px(0.), px(-4.)),
            grab: true,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            menu_size,
        ))),
        titlebar: None,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("org.basr.Menu".into()),
        ..Default::default()
    }
}

/// Menu callbacks that edit the composer must use its owning window.
pub(super) fn in_parent(
    parent: AnyWindowHandle,
    window: &mut Window,
    cx: &mut App,
    callback: impl FnOnce(&mut Window, &mut App),
) {
    if window.window_handle() == parent {
        callback(window, cx);
    } else {
        let _ = parent.update(cx, |_, window, cx| callback(window, cx));
    }
}
