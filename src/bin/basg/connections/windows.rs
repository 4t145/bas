use std::{collections::HashMap, path::PathBuf};

use bas::service::tls::ConnectionOptions as Target;
use gpui_kit::{
    App, AppContext, Bounds, Global, WindowBounds, WindowDecorations, WindowHandle, WindowKind,
    WindowOptions, component::Root, prelude::*, px, size, transparent_black,
};
use tokio::runtime::Handle;

mod host;
pub(crate) use host::{Host as WorkspaceHost, workspace_view};

#[derive(Clone)]
struct Launch {
    runtime: Handle,
    cwd: PathBuf,
    compact: bool,
}
impl Global for Launch {}

#[derive(Default)]
struct Windows(HashMap<Target, WindowHandle<Root>>);
impl Global for Windows {}

pub(crate) fn init_windows(runtime: Handle, cwd: PathBuf, compact: bool, cx: &mut App) {
    cx.set_global(Launch {
        runtime,
        cwd,
        compact,
    });
    cx.set_global(Windows::default());
}

pub(crate) fn register(target: Target, handle: WindowHandle<Root>, cx: &mut App) {
    if !cx.has_global::<Windows>() {
        cx.set_global(Windows::default());
    }
    cx.update_global::<Windows, _>(|windows, _| {
        windows.0.insert(target, handle);
    });
    cx.update_global::<crate::agents::Changes, _>(|changes, _| {
        changes.target = Some(handle.window_id());
        changes.server = None;
        changes.select = None;
        changes.ready = None;
    });
}

pub(crate) fn window_options(compact: bool, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            crate::workspace::initial_size(compact),
            cx,
        ))),
        kind: WindowKind::Normal,
        titlebar: cfg!(target_os = "macos").then(gpui_kit::component::TitleBar::title_bar_options),
        app_owns_titlebar_drag: cfg!(target_os = "macos"),
        window_decorations: Some(WindowDecorations::Client),
        window_background: cx.global::<crate::theme::Appearance>().background,
        window_min_size: Some(size(px(420.), px(80.))),
        app_id: Some("org.bas.Gui".into()),
        ..Default::default()
    }
}

/// Opens or activates this backend's window without repurposing another backend's drafts.
///
/// # Errors
/// Returns a missing-runtime or platform window error.
pub(crate) fn open_current(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    let target = super::current(cx).ok_or_else(|| anyhow::anyhow!("No backend selected"))?;
    open_for(target, cx)
}

/// # Errors
/// Returns a missing-runtime or platform window error without changing management context.
pub(crate) fn open_for(target: Target, cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    let launch = cx
        .try_global::<Launch>()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("The workspace runtime is unavailable"))?;
    if let Some(handle) = cx
        .try_global::<Windows>()
        .and_then(|windows| windows.0.get(&target))
        .copied()
        && let Ok(result) = handle.update(cx, |root, window, cx| {
            if let Ok(host) = root.view().clone().downcast::<WorkspaceHost>() {
                host.update(cx, |host, cx| host.select(target.clone(), window, cx))?;
            }
            window.activate_window();
            anyhow::Ok(())
        })
    {
        result?;
        register(target, handle, cx);
        return Ok(handle);
    }
    let handle = cx.open_window(window_options(launch.compact, cx), |window, cx| {
        let view = cx.new(|cx| {
            crate::workspace::Workspace::new(
                target.clone(),
                launch.cwd,
                launch.compact,
                &launch.runtime,
                window,
                cx,
            )
        });
        let view = workspace_view(view, window, cx);
        cx.new(|cx| {
            crate::theme::window_root(view, window, cx)
                .bordered(false)
                .bg(transparent_black())
        })
    })?;
    register(target, handle, cx);
    Ok(handle)
}

pub(super) fn switch_in(
    target: Target,
    handle: gpui_kit::AnyWindowHandle,
    cx: &mut App,
) -> anyhow::Result<()> {
    let handle = handle
        .downcast::<Root>()
        .ok_or_else(|| anyhow::anyhow!("The workspace window is unavailable"))?;
    handle.update(cx, |root, window, cx| {
        let host = root
            .view()
            .clone()
            .downcast::<WorkspaceHost>()
            .map_err(|_| anyhow::anyhow!("This window does not contain a workspace"))?;
        host.update(cx, |host, cx| host.select(target.clone(), window, cx))
    })??;
    register(target, handle, cx);
    Ok(())
}
