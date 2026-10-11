//! Native session management and agent conversations for BASR.

mod agent_icon;
mod agent_metadata;
mod application;
mod approval;
mod assets;
mod attachment;
mod authentication;
mod branding;
mod clipboard;
mod connection;
mod conversation;
mod desktop;
mod http;
mod input_geometry;
mod settings;
mod settings_window;
mod theme;
mod tooltip;
mod workspace;

use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context as _, Result};
use clap::Parser;
use gpui_kit::{
    AppContext, Bounds, WindowBounds, WindowDecorations, WindowOptions,
    component::{Root, StyledExt},
    prelude::*,
    px, size, transparent_black,
};

const NETWORK_THREADS: usize = 2;

#[derive(Parser)]
#[command(name = "basrw", about = "BASR 会话与 Agent 工作区")]
struct Args {
    #[arg(long, env = "BASR_SERVER", default_value = basr::service::DEFAULT_ADDRESS)]
    server: SocketAddr,
    /// Default working directory for newly created sessions.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Open the minimal input popup (the default).
    #[arg(long, conflicts_with = "window")]
    compact: bool,
    /// Open the workspace and thread window instead of the minimal popup.
    #[arg(long)]
    window: bool,
    /// Override the DMS-generated Zed theme file; changes are reloaded live.
    #[arg(long)]
    theme_file: Option<PathBuf>,
    /// Unified BASR settings (default: basr/settings.json in the user config directory).
    #[arg(long, env = "BASR_SETTINGS_FILE")]
    settings_file: Option<PathBuf>,
    #[cfg(feature = "gui-tests")]
    #[arg(long, hide = true, value_parser = ["empty", "startup", "agent-menu", "sample", "layout", "connecting", "disconnected", "menus", "files", "images", "picker", "thoughts", "completed", "reply-preview", "attachments", "attachment-preview", "multiline", "multiline-hover", "tool-details", "usage", "usage-details", "usage-hover", "clipboard", "settings"])]
    preview_panel: Option<String>,
    #[cfg(all(feature = "gui-tests", target_os = "linux"))]
    #[arg(long, hide = true)]
    test_wayland_clipboard: bool,
}

fn main() -> Result<()> {
    settings::run_helper_if_requested();
    let args = Args::parse();
    #[cfg(all(feature = "gui-tests", target_os = "linux"))]
    if args.test_wayland_clipboard {
        return clipboard::wayland::check();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(NETWORK_THREADS)
        .enable_all()
        .build()
        .context("cannot start the GUI network runtime")?;
    let handle = runtime.handle().clone();
    let cwd = args
        .cwd
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    gpui_kit::application()
        .with_http_client(http::client(&handle)?)
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            settings::init_checks(args.server, handle.clone(), cx);
            agent_metadata::init(args.server, handle.clone(), cx);
            theme::init(args.theme_file, args.settings_file, cx);
            theme::watch(cx);
            application::init(cx);
            workspace::bind_keys(cx);
            settings_window::bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            #[cfg(feature = "gui-tests")]
            if args.preview_panel.as_deref() == Some("settings") {
                if let Err(error) = settings_window::open(cx) {
                    tracing::error!(%error, "cannot open the BASR settings window");
                    cx.quit();
                }
                return;
            }
            let bounds = Bounds::centered(None, workspace::initial_size(!args.window), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: cfg!(target_os = "macos")
                    .then(gpui_kit::component::TitleBar::title_bar_options),
                app_owns_titlebar_drag: cfg!(target_os = "macos"),
                window_decorations: Some(WindowDecorations::Client),
                window_background: cx.global::<theme::Appearance>().background,
                window_min_size: Some(size(px(420.), px(80.))),
                app_id: Some("org.basr.Gui".into()),
                ..Default::default()
            };
            if let Err(error) = cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| {
                    #[cfg(feature = "gui-tests")]
                    if let Some(preview) = &args.preview_panel {
                        return workspace::Workspace::preview(
                            preview,
                            !args.window,
                            &handle,
                            window,
                            cx,
                        );
                    }
                    workspace::Workspace::new(args.server, cwd, !args.window, &handle, window, cx)
                });
                cx.new(|cx| {
                    Root::new(view, window, cx)
                        .refine_style(&theme::font_style(cx))
                        .bordered(false)
                        .bg(transparent_black())
                })
            }) {
                tracing::error!(%error, "cannot open the BASR window");
                cx.quit();
            }
        });
    Ok(())
}
