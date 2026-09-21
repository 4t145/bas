//! One independently-lived settings window per application.

use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, Focusable, Global, Render, Subscription,
    TestSupportExt, WeakEntity, Window, WindowBounds, WindowDecorations, WindowHandle,
    WindowOptions, actions,
    component::{ActiveTheme, Root, StyledExt, TitleBar},
    div,
    prelude::*,
    px, size,
};

use crate::{
    settings::{Close, Settings},
    theme,
};

const WIDTH: f32 = 960.;
const HEIGHT: f32 = 720.;
const MIN_WIDTH: f32 = 720.;
const MIN_HEIGHT: f32 = 480.;

actions!(settings_window, [CloseWindow]);

struct OpenWindow {
    handle: WindowHandle<Root>,
    settings: WeakEntity<Settings>,
}
impl Global for OpenWindow {}

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([gpui_kit::KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-w"
        },
        CloseWindow,
        Some("BasrSettings"),
    )]);
}

/// # Errors
/// Returns a platform error if a new native window cannot be created.
pub(crate) fn open(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    if let Some(handle) = current(cx)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Ok(handle);
    }
    let mut settings = None;
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(WIDTH), px(HEIGHT)),
                cx,
            ))),
            window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
            window_decorations: Some(WindowDecorations::Client),
            window_background: cx.global::<theme::Appearance>().background,
            app_id: Some("org.basr.Gui".into()),
            ..TitleBar::window_options()
        },
        |window, cx| {
            window.set_window_title("设置 — BASR");
            let host = cx.new(|cx| Host::new(window, cx));
            settings = Some(host.read(cx).settings.downgrade());
            cx.new(|cx| Root::new(host, window, cx))
        },
    )?;
    cx.set_global(OpenWindow {
        handle,
        settings: settings.expect("the settings window builder created its host"),
    });
    Ok(handle)
}

pub(crate) fn current(cx: &App) -> Option<WindowHandle<Root>> {
    cx.try_global::<OpenWindow>().map(|state| state.handle)
}

/// Persist valid edits even when the workspace which opened settings is gone.
pub(crate) fn flush_pending(cx: &mut App) {
    let Some(state) = cx.try_global::<OpenWindow>() else {
        return;
    };
    let handle = state.handle;
    let settings = state.settings.clone();
    let _ = handle.update(cx, |_, window, cx| {
        let _ = settings.update(cx, |settings, cx| settings.flush_pending(window, cx));
    });
}

struct Host {
    settings: Entity<Settings>,
    _close: Subscription,
}

impl Host {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.new(|cx| Settings::new(window, cx));
        let weak = settings.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak.update(cx, |settings, cx| settings.flush_pending(window, cx));
            true
        });
        let close = cx.subscribe_in(&settings, window, |this, _, _: &Close, window, cx| {
            this.close(window, cx);
        });
        settings.read(cx).focus_handle(cx).focus(window, cx);
        window.activate_window();
        Self {
            settings,
            _close: close,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |settings, cx| settings.flush_pending(window, cx));
        window.remove_window();
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("settings-window")
            .test_support()
            .key_context("BasrSettings")
            .size_full()
            .flex()
            .flex_col()
            .refine_style(&theme::font_style(cx))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| this.close(window, cx)))
            .child(
                TitleBar::new()
                    .child("设置 — BASR")
                    .on_close_window(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .child(div().flex_1().min_h_0().child(self.settings.clone()))
    }
}
