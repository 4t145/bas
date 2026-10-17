//! One independent history window per daemon, with its own connection lifetime.

use super::*;
use crate::{theme, ui::window_controls::TitleBar};
use gpui_kit::{
    AppContext, Bounds, ClickEvent, Global, Render, TestSupportExt, WeakEntity, WindowBounds,
    WindowDecorations, WindowHandle, WindowKind, WindowOptions, actions,
    component::{ActiveTheme, Icon, Root},
    div,
    prelude::*,
    px, size,
};

const WIDTH: f32 = 1_000.;
const HEIGHT: f32 = 720.;
const MIN_WIDTH: f32 = 780.;
const MIN_HEIGHT: f32 = 480.;

actions!(history_window, [CloseWindow]);

#[derive(Clone)]
struct OpenWindow {
    handle: WindowHandle<Root>,
    browser: WeakEntity<Browser>,
}

#[derive(Default)]
struct Windows(BTreeMap<SocketAddr, OpenWindow>);
impl Global for Windows {}

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([gpui_kit::KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-w"
        },
        CloseWindow,
        Some("BasAgentHistory"),
    )]);
}

/// # Errors
/// Reports an unavailable network runtime or native window creation failure.
pub(crate) fn open(request: OpenRequest, cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    if let Some(handle) = reopen(request.clone(), cx) {
        return Ok(handle);
    }
    let service = cx
        .try_global::<crate::agents::catalog::Service>()
        .ok_or_else(|| anyhow::anyhow!(crate::i18n::tr!("agent-history-unavailable")))?;
    let (connection, events) = Connection::new(request.server, service.runtime());
    open_connected(request, connection, events, cx)
}

fn reopen(request: OpenRequest, cx: &mut App) -> Option<WindowHandle<Root>> {
    let open = cx.try_global::<Windows>()?.0.get(&request.server)?.clone();
    open.handle
        .update(cx, |_, window, cx| {
            open.browser
                .update(cx, |browser, cx| browser.configure(request, cx))?;
            window.activate_window();
            Ok::<_, anyhow::Error>(())
        })
        .ok()?
        .ok()?;
    Some(open.handle)
}

/// # Errors
/// Returns a platform error without changing any conversation or draft.
pub(crate) fn open_connected(
    request: OpenRequest,
    connection: Connection,
    events: tokio::sync::mpsc::Receiver<Update>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Root>> {
    if let Some(handle) = reopen(request.clone(), cx) {
        return Ok(handle);
    }
    let server = request.server;
    let mut browser = None;
    let handle = cx.open_window(window_options(cx), |window, cx| {
        window.set_window_title(crate::i18n::tr!("agent-history-title"));
        let history = cx.new(|cx| Browser::new(request, connection, events, window, cx));
        browser = Some(history.downgrade());
        history.update(cx, |history, cx| {
            history
                .search
                .update(cx, |search, cx| search.focus(window, cx))
        });
        let host = cx.new(|_| Host { browser: history });
        window.activate_window();
        cx.new(|cx| theme::window_root(host, window, cx))
    })?;
    if !cx.has_global::<Windows>() {
        cx.set_global(Windows::default());
    }
    cx.update_global::<Windows, _>(|windows, _| {
        windows.0.insert(
            server,
            OpenWindow {
                handle,
                browser: browser.expect("history window created its browser"),
            },
        );
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

#[cfg(all(test, feature = "gui-tests"))]
pub(super) fn browser(server: SocketAddr, cx: &App) -> WeakEntity<Browser> {
    cx.global::<Windows>().0[&server].browser.clone()
}

struct Host {
    browser: Entity<Browser>,
}

impl Host {
    fn close(&mut self, _: &ClickEvent, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }
    fn close_action(&mut self, _: &CloseWindow, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(crate::i18n::tr!("agent-history-title"));
        gds_uim::el! { #div {
            id: "agent-history-window"; test_support; key_context: "BasAgentHistory";
            size_full; flex; flex_col; bg: $cx.theme().background; text_color: $cx.theme().foreground;
            on_action: $cx.listener(Self::close_action);
        } [
            TitleBar { on_close_window: $cx.listener(Self::close); } [
                #div { flex; items_center; gap_2; } [
                    Icon(ICON) { size: crate::ui::ICON.px; },
                    $crate::i18n::tr!("agent-history-title"),
                ],
            ],
            #div { flex_1; min_h_0; } [$self.browser.clone()],
        ] }
    }
}
