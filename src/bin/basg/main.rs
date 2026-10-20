//! Native session management and agent conversations for Bas.

mod agents;
mod application;
mod assets;
mod attachment;
mod audio;
mod authentication;
mod branding;
mod clipboard;
mod configuration;
mod connection;
mod connections;
mod conversation;
mod diagnostics;
mod elicitation;
mod history;
mod http;
mod i18n;
mod settings;
mod submission;
mod theme;
mod ui;
mod workspace;

#[cfg(test)]
mod tests;

use bas::service::tls::ConnectionOptions as Target;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use gpui_kit::{AppContext, prelude::*, transparent_black};

const NETWORK_THREADS: usize = 2;

#[derive(Parser)]
#[command(name = "basg", about = "BasG 会话与 Agent 工作区")]
struct Args {
    /// Override the saved connection: local, unix:///path, pipe://NAME, or tcp://HOST[:PORT].
    #[arg(long, env = "BAS_SERVER")]
    server: Option<Target>,
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
    /// Unified Bas settings (default: bas/settings.json in the user config directory).
    #[arg(long, env = "BAS_SETTINGS_FILE")]
    settings_file: Option<PathBuf>,
    #[cfg(feature = "gui-tests")]
    #[arg(long, hide = true, value_parser = ["empty", "startup", "agent-menu", "sample", "layout", "inline-wrap", "connecting", "disconnected", "menus", "files", "images", "picker", "thoughts", "completed", "reply-preview", "attachments", "attachment-preview", "multiline", "multiline-hover", "tool-details", "activity-artifacts", "activity-artifacts-collapsed", "thread-feedback", "markdown-actions", "markdown-copy-hover", "subtle-actions", "turn-navigation", "turn-navigation-hover", "turn-navigation-many", "usage", "usage-details", "usage-hover", "agent-hover", "attachment-hint", "app-menu", "thread-menu", "clipboard", "settings", "settings-agents", "settings-connections", "agent-onboarding"])]
    preview_panel: Option<String>,
    #[cfg(all(feature = "gui-tests", target_os = "linux"))]
    #[arg(long, hide = true)]
    test_wayland_clipboard: bool,
    #[cfg(feature = "gui-tests")]
    #[arg(long, hide = true)]
    preview_logs: bool,
}

fn main() -> Result<()> {
    authentication::terminal::run_helper_if_requested();
    let args = Args::parse();
    #[cfg(all(feature = "gui-tests", target_os = "linux"))]
    if args.test_wayland_clipboard {
        return clipboard::wayland::check();
    }
    let logs = diagnostics::init_tracing();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        build = env!("BAS_BUILD_TAG"),
        "Frontend started"
    );
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
            diagnostics::init(logs, cx);
            configuration::init(args.settings_file, cx);
            let profiles = connections::shared(cx);
            let server = match profiles.read(cx).startup_target(args.server) {
                Ok(server) => server,
                Err(error) => {
                    tracing::error!(%error, "cannot select frontend connection; refusing to fall back");
                    cx.quit();
                    return;
                }
            };
            agents::init(server.clone(), handle.clone(), cx);
            agents::metadata::init(server.clone(), handle.clone(), cx);
            connections::init_windows(handle.clone(), cwd.clone(), !args.window, cx);
            theme::init(args.theme_file, cx);
            theme::watch(cx);
            application::init(cx);
            workspace::bind_keys(cx);
            settings::window::bind_keys(cx);
            history::window::bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            #[cfg(feature = "gui-tests")]
            if args.preview_logs {
                if let Err(error) = diagnostics::window::fixture::open(cx) {
                    tracing::error!(%error, "cannot open log preview");
                    cx.quit();
                }
                return;
            }
            #[cfg(feature = "gui-tests")]
            if let Some(
                panel @ ("settings"
                | "settings-agents"
                | "settings-connections"
                | "agent-onboarding"),
            ) = args.preview_panel.as_deref()
            {
                let opened = match panel {
                    "settings-agents" => settings::window::open_agent("embedded", cx),
                    "settings-connections" => settings::window::open_connections(cx),
                    "agent-onboarding" => settings::window::open_catalog(cx),
                    _ => settings::window::open(cx),
                };
                if let Err(error) = opened {
                    tracing::error!(%error, "cannot open the BasG settings window");
                    cx.quit();
                }
                return;
            }
            let options = connections::window_options(!args.window, cx);
            let opened = cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| {
                    #[cfg(feature = "gui-tests")]
                    if let Some(preview) = &args.preview_panel {
                        return workspace::Workspace::preview(preview, !args.window, window, cx);
                    }
                    workspace::Workspace::new(
                        server.clone(),
                        cwd,
                        !args.window,
                        &handle,
                        window,
                        cx,
                    )
                });
                let view = connections::workspace_view(view, window, cx);
                cx.new(|cx| {
                    theme::window_root(view, window, cx)
                        .bordered(false)
                        .bg(transparent_black())
                })
            });
            match opened {
                Ok(window) => connections::register(server, window, cx),
                Err(error) => {
                    tracing::error!(%error, "cannot open the BasG window");
                    cx.quit();
                }
            }
        });
    Ok(())
}
