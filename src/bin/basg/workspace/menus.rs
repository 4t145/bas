//! Keep compact-window menus on a separate surface, using the component menu itself.

use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    AnyWindowHandle, Bounds, DismissEvent, FocusHandle, Focusable, MouseButton,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, anchored, canvas,
    component::menu::{DropdownMenu, PopupMenu},
    deferred, point,
    popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions},
};

use super::*;

use crate::ui::{MENU_MAX_HEIGHT, MENU_ROW_HEIGHT, MENU_WIDTH};
const MENU_PADDING: f32 = 12.;
pub(super) const SHADOW_PADDING: f32 = 12.;

pub(super) type Builder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

pub(super) struct OpenMenu {
    pub window: Option<AnyWindowHandle>,
    pub content: Entity<PopupMenu>,
    pub size: Size<Pixels>,
    trigger: Bounds<Pixels>,
    previous_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

struct MenuSurface {
    content: Entity<PopupMenu>,
}

#[derive(Clone)]
struct MenuTrigger {
    bounds: Rc<Cell<Bounds<Pixels>>>,
    rows: usize,
    disabled: bool,
    build: Builder,
}

impl Render for MenuSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        el! {
            .div {
                p: SHADOW_PADDING.px;
                on_action: $cx.listener(Self::on_settings);
                on_action: $cx.listener(Self::on_quit);
                on_prepaint: Self::resize_to_content;
            } [~$(self.content)]
        }
    }
}

impl MenuSurface {
    fn on_settings(
        &mut self,
        _: &crate::application::OpenSettings,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_for_action(cx);
    }

    fn on_quit(&mut self, _: &crate::application::Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_for_action(cx);
    }

    fn resize_to_content(bounds: Bounds<Pixels>, window: &mut Window, _: &mut App) {
        let desired = size(window.bounds().size.width, bounds.size.height.ceil());
        if desired != window.bounds().size {
            window.resize(desired);
        }
    }

    fn dismiss_for_action(&self, cx: &mut Context<Self>) {
        self.content.update(cx, |_, cx| cx.emit(DismissEvent));
        cx.propagate();
    }
}

impl Workspace {
    pub(super) fn open_context_menu(
        &mut self,
        position: gpui_kit::Point<Pixels>,
        rows: usize,
        build: Builder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(window, cx);
        self.open_menu(
            Bounds::new(position, size(px(0.), px(0.))),
            rows,
            build,
            window,
            cx,
        );
    }

    pub(super) fn inline_context_menu(&self) -> Option<AnyElement> {
        let menu = self
            .window
            .menu
            .as_ref()
            .filter(|menu| !self.window.compact && menu.window.is_none())?;
        Some(
            deferred(el! { .anchored {
                position: $menu.trigger.origin;
                snap_to_window_with_margin: 8.0.px;
            } [~$(menu.content)] })
            .with_priority(gpui_kit::base::POPUP_PRIORITY)
            .into_any_element(),
        )
    }

    pub(super) fn menu_button(
        &self,
        button: Button,
        rows: usize,
        disabled: bool,
        build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.window.compact {
            return el! { button {
                dropdown_menu: move (menu, window, cx) => (menu, window, cx).build.ui::menu;
            } }
            .into_any_element();
        }
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        let trigger = MenuTrigger {
            bounds,
            rows,
            disabled,
            build: Rc::new(build),
        };
        let keyboard_trigger = trigger.clone();
        let trigger = el! {
            button {
                on_mouse_down: MouseButton::Left, $cx.listener(move |this, _, window, cx| {
                    this.on_menu_trigger(&trigger, window, cx);
                });
                on_key_down: $cx.listener(move |this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    this.on_menu_key(event, &keyboard_trigger, window, cx);
                });
            }
        };
        el! {
            .div { relative; flex; flex_shrink_0; } [
                trigger,
                (move (bounds, _, _) => $measured.set(bounds), $|_, _, _, _| {}).canvas {
                    absolute; size_full;
                }
            ]
        }
        .into_any_element()
    }

    fn on_menu_trigger(
        &mut self,
        trigger: &MenuTrigger,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !trigger.disabled {
            window.prevent_default();
            cx.stop_propagation();
            self.open_menu(
                trigger.bounds.get(),
                trigger.rows,
                trigger.build.clone(),
                window,
                cx,
            );
        }
    }

    fn on_menu_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        trigger: &MenuTrigger,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
            self.on_menu_trigger(trigger, window, cx);
        }
    }

    fn on_menu_dismissed(
        &mut self,
        _: &Entity<PopupMenu>,
        _: &DismissEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(window, cx);
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
            .window
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
            .unwrap_or(size(px(MENU_WIDTH * 2.), px(MENU_MAX_HEIGHT * 2.)));
        let menu_size = size(
            px(MENU_WIDTH).min(screen.width * 0.8),
            px(rows as f32 * MENU_ROW_HEIGHT + MENU_PADDING)
                .min(px(MENU_MAX_HEIGHT))
                .min(screen.height * 0.6),
        );
        let previous_focus = window.focused(cx);
        let content = PopupMenu::build(window, cx, |menu, window, cx| {
            el! { (menu, window, cx).build.ui::menu {
                min_w: $menu_size.width; max_w: $menu_size.width; max_h: $menu_size.height;
            } }
        });
        let parent = window.window_handle();
        let menu = content.clone();
        let native =
            match cx.open_window(popup_options(parent, trigger, menu_size), |window, cx| {
                menu.focus_handle(cx).focus(window, cx);
                let surface = cx.new(|_| MenuSurface { content: menu });
                cx.new(|cx| {
                    el! { (surface, window, cx).theme::window_root {
                        bg: .gpui_kit::transparent_black;
                        bordered: false; window_shadow_size: 0.0.px;
                    } }
                })
            }) {
                Ok(handle) => Some(handle.into()),
                Err(error) => {
                    tracing::debug!(%error, "using an inline menu on this platform");
                    content.focus_handle(cx).focus(window, cx);
                    None
                }
            };
        let dismissed = cx.subscribe_in(&content, window, Self::on_menu_dismissed);
        let view = cx.entity().downgrade();
        let closed = cx.on_window_closed(move |cx, id| {
            if native.is_some_and(|popup: AnyWindowHandle| popup.window_id() == id) {
                let _ = parent.update(cx, |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.close_menu(window, cx));
                });
            }
        });
        self.window.menu = Some(OpenMenu {
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
        let Some(menu) = self.window.menu.take() else {
            return;
        };
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
            .window
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
            offset: point(px(-SHADOW_PADDING), px(SHADOW_PADDING - 4.)),
            grab: true,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(
                menu_size.width + px(SHADOW_PADDING * 2.),
                menu_size.height + px(SHADOW_PADDING * 2.),
            ),
        ))),
        titlebar: None,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("org.bas.Menu".into()),
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
