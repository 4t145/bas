//! One independently-lived settings window per application.

use gds_uim::el;
use gpui_kit::{
    App, AppContext, Bounds, ClickEvent, Context, Entity, Focusable, Global, Render,
    TestSupportExt, WeakEntity, Window, WindowBounds, WindowDecorations, WindowHandle, WindowKind,
    WindowOptions, actions,
    component::{ActiveTheme, Root},
    div,
    prelude::*,
    px, size,
};

use crate::ui::window_controls::TitleBar;
use crate::{i18n::tr, settings::Settings, theme};

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
    super::connections::bind_keys(cx);
    cx.bind_keys([gpui_kit::KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-w"
        },
        CloseWindow,
        Some("BasSettings"),
    )]);
}

/// # Errors
/// Returns a platform error if a new native window cannot be created.
pub(crate) fn open(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    if let Some(handle) = current(cx)
        && handle
            .update(cx, |_, window, cx| {
                // A workspace may change management context immediately before
                // revealing an agent, before global observers have been delivered.
                let settings = cx.global::<OpenWindow>().settings.clone();
                let _ = settings.update(cx, |settings, cx| settings.on_backend_changed(window, cx));
                window.activate_window();
            })
            .is_ok()
    {
        return Ok(handle);
    }
    let mut settings = None;
    let handle = cx.open_window(window_options(cx), |window, cx| {
        window.set_window_title(crate::i18n::tr!("settings-window-title"));
        let host = cx.new(|cx| Host::new(window, cx));
        settings = Some(host.read(cx).settings.downgrade());
        cx.new(|cx| theme::window_root(host, window, cx))
    })?;
    cx.set_global(OpenWindow {
        handle,
        settings: settings.expect("the settings window builder created its host"),
    });
    Ok(handle)
}

fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        kind: WindowKind::Floating,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(WIDTH), px(HEIGHT)),
            cx,
        ))),
        window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
        window_decorations: Some(WindowDecorations::Client),
        window_background: cx.global::<theme::Appearance>().background,
        app_id: Some("org.bas.Gui".into()),
        ..TitleBar::window_options()
    }
}

pub(crate) fn current(cx: &App) -> Option<WindowHandle<Root>> {
    cx.try_global::<OpenWindow>().map(|state| state.handle)
}

/// # Errors
/// Returns an error if the settings window cannot be opened or updated.
pub(crate) fn open_connections(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    let handle = open(cx)?;
    let settings = cx.global::<OpenWindow>().settings.clone();
    handle.update(cx, |_, window, cx| {
        settings.update(cx, |settings, cx| {
            settings.select_category(super::Category::Connections, window, cx);
        })
    })??;
    Ok(handle)
}

/// Opens a local agent's settings and explicitly refreshes its ACP auth methods.
///
/// # Errors
/// Returns an error if the settings window cannot be opened or updated.
pub(crate) fn open_agent(agent: &str, cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    let handle = open(cx)?;
    let settings = cx.global::<OpenWindow>().settings.clone();
    handle.update(cx, |_, window, cx| {
        settings.update(cx, |settings, cx| settings.show_agent(agent, window, cx))
    })??;
    Ok(handle)
}

/// # Errors
/// Returns an error if the settings window cannot be opened or updated.
pub(crate) fn open_agent_management(
    agent: &str,
    target: Option<bas::service::tls::ConnectionOptions>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Root>> {
    if let Some(target) = target {
        crate::connections::manage(target, cx);
    } else {
        anyhow::ensure!(
            crate::connections::current(cx).is_none(),
            "The originating standalone settings context is no longer active"
        );
    }
    let handle = open(cx)?;
    let settings = cx.global::<OpenWindow>().settings.clone();
    handle.update(cx, |_, window, cx| {
        settings.update(cx, |settings, cx| {
            let index = settings
                .agents
                .read(cx)
                .configuration_entries()
                .iter()
                .position(|entry| entry.id == agent);
            settings.select_agents(index, window, cx);
        })
    })??;
    Ok(handle)
}

/// # Errors
/// Returns an error if the independent agent onboarding window cannot be opened.
pub(crate) fn open_catalog(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    super::agents::window::open_catalog(cx)
}

/// Persist valid edits even when the workspace which opened settings is gone.
pub(crate) fn flush_pending(cx: &mut App) {
    let Some(state) = cx.try_global::<OpenWindow>() else {
        return;
    };
    let handle = state.handle;
    let settings = state.settings.clone();
    let _ = handle.update(cx, |_, window, cx| {
        let _ = settings.update(cx, |settings, cx| settings.prepare_to_close(window, cx));
    });
}

struct Host {
    settings: Entity<Settings>,
}

impl Host {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.new(|cx| Settings::new(window, cx));
        let weak = settings.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak.update(cx, |settings, cx| settings.prepare_to_close(window, cx));
            true
        });
        settings.read(cx).focus_handle(cx).focus(window, cx);
        window.activate_window();
        Self { settings }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |settings, cx| settings.prepare_to_close(window, cx));
        window.remove_window();
    }

    fn on_close_action(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn on_close_click(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }
}

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(crate::i18n::tr!("settings-window-title"));
        el! {
            .div {
                # "settings-window";
                test_support;
                key_context: "BasSettings";
                size_full; flex; flex_col;
                bg: $cx.theme().background;
                text_color: $cx.theme().foreground;
                on_action: (cx, Self::on_close_action).Context::listener;
            } [
                .TitleBar::new {
                    on_close_window: (cx, Self::on_close_click).Context::listener;
                } [tr!("settings-window-title")],
                .div { flex_1; min_h_0; } [~$(self.settings)],
            ]
        }
    }
}
