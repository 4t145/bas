//! Application-only windows; these never send commands to basrd.

use gpui_kit::{
    App, AppContext, Bounds, Context, Global, KeyBinding, Render, TestSupportExt, Window,
    WindowBounds, WindowDecorations, WindowHandle, WindowOptions, actions,
    component::{ActiveTheme, Root, StyledExt, TitleBar, button::*},
    div,
    prelude::*,
    px, size,
};

use crate::{branding, settings_window, theme};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(crate) const RELEASES_URL: &str = "https://github.com/4t145/basr/releases";
const HOME_URL: &str = "https://github.com/4t145/basr";
const WIDTH: f32 = 480.;
const HEIGHT: f32 = 420.;
const QUIT_NOTICE: &str =
    "退出会丢失未发送的草稿和本地队列。后台已提交的任务会继续运行；不会取消任务或停止 basrd。";

actions!(application, [OpenSettings, Quit]);

struct ActionsRegistered;
impl Global for ActionsRegistered {}

pub(crate) fn init(cx: &mut App) {
    if cx.has_global::<ActionsRegistered>() {
        return;
    }
    cx.set_global(ActionsRegistered);
    let (settings, quit) = if cfg!(target_os = "macos") {
        ("cmd-,", "cmd-q")
    } else {
        ("ctrl-,", "ctrl-q")
    };
    cx.bind_keys([
        KeyBinding::new(settings, OpenSettings, None),
        KeyBinding::new(quit, Quit, None),
    ]);
    cx.on_action(|_: &OpenSettings, cx| {
        if let Err(error) = settings_window::open(cx) {
            tracing::error!(%error, "cannot open settings");
        }
    });
    cx.on_action(|_: &Quit, cx| {
        if let Err(error) = open_quit(cx) {
            tracing::error!(%error, "cannot open quit confirmation");
        }
    });
}

#[derive(Clone, Copy)]
enum InfoWindow {
    About,
    Quit,
    Message,
    Confirmation,
}

#[derive(Default)]
struct OpenWindows {
    about: Option<WindowHandle<Root>>,
    quit: Option<WindowHandle<Root>>,
    message: Option<WindowHandle<Root>>,
    confirmation: Option<WindowHandle<Root>>,
}
impl Global for OpenWindows {}

pub(crate) struct Confirmation {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
}

/// Shows an error independently of any pending confirmation.
///
/// # Errors
/// Returns a platform error if the message window cannot be created.
pub(crate) fn open_message(
    title: &str,
    message: &str,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Root>> {
    // Replace only the previous informational message, never a confirmation.
    if let Some(handle) = cx
        .try_global::<OpenWindows>()
        .and_then(|state| state.message)
    {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
    open_host(
        Host {
            kind: InfoWindow::Message,
            title: title.into(),
            message: message.into(),
            confirm_label: String::new(),
            on_confirm: None,
        },
        cx,
    )
}

/// Repeated requests focus the pending confirmation and discard the new request.
/// Cancel/close drops its callback without executing it.
///
/// # Errors
/// Returns a platform error if the confirmation window cannot be created.
pub(crate) fn open_confirmation(
    request: Confirmation,
    on_confirm: impl FnOnce(&mut App) + 'static,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Root>> {
    open_host(
        Host {
            kind: InfoWindow::Confirmation,
            title: request.title,
            message: request.message,
            confirm_label: request.confirm_label,
            on_confirm: Some(Box::new(on_confirm)),
        },
        cx,
    )
}

/// Opens the release page for manual inspection; no version comparison is performed.
pub(crate) fn open_releases(cx: &mut App) {
    cx.open_url(RELEASES_URL);
}

/// # Errors
/// Returns a platform error if the About window cannot be created.
pub(crate) fn open_about(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    open_host(Host::new(InfoWindow::About), cx)
}

/// # Errors
/// Returns a platform error if the quit confirmation window cannot be created.
pub(crate) fn open_quit(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    open_host(Host::new(InfoWindow::Quit), cx)
}

impl InfoWindow {
    fn title(self) -> &'static str {
        match self {
            Self::About => "关于 BASR",
            Self::Quit => "退出 BASR？",
            Self::Message => "BASR",
            Self::Confirmation => "确认操作",
        }
    }

    fn slot(self, windows: &mut OpenWindows) -> &mut Option<WindowHandle<Root>> {
        match self {
            Self::About => &mut windows.about,
            Self::Quit => &mut windows.quit,
            Self::Message => &mut windows.message,
            Self::Confirmation => &mut windows.confirmation,
        }
    }
}

/// # Errors
/// Returns a platform error when opening a native window fails.
fn open_host(host: Host, cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    let kind = host.kind;
    if !cx.has_global::<OpenWindows>() {
        cx.set_global(OpenWindows::default());
    }
    if let Some(handle) = *kind.slot(cx.global_mut::<OpenWindows>())
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Ok(handle);
    }
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(WIDTH), px(HEIGHT)),
                cx,
            ))),
            window_min_size: Some(size(px(WIDTH), px(HEIGHT))),
            window_decorations: Some(WindowDecorations::Client),
            window_background: cx.global::<theme::Appearance>().background,
            app_id: Some("org.basr.Gui".into()),
            ..TitleBar::window_options()
        },
        |window, cx| {
            window.set_window_title(&host.title);
            let host = cx.new(|_| host);
            window.activate_window();
            cx.new(|cx| Root::new(host, window, cx))
        },
    )?;
    *kind.slot(cx.global_mut::<OpenWindows>()) = Some(handle);
    Ok(handle)
}

type ConfirmCallback = Box<dyn FnOnce(&mut App)>;

struct Host {
    kind: InfoWindow,
    title: String,
    message: String,
    confirm_label: String,
    on_confirm: Option<ConfirmCallback>,
}

impl Host {
    fn new(kind: InfoWindow) -> Self {
        Self {
            kind,
            title: kind.title().into(),
            message: String::new(),
            confirm_label: String::new(),
            on_confirm: None,
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let callback = self.on_confirm.take();
        Self::close(window);
        if let Some(callback) = callback {
            callback(cx);
        }
    }

    fn close(window: &mut Window) {
        window.remove_window();
    }

    fn message(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_6()
            .p_6()
            .child(
                div()
                    .id("application-message-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .whitespace_normal()
                    .child(crate::tooltip::wrap_tokens(&self.message)),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("message-close")
                            .label(if matches!(self.kind, InfoWindow::Confirmation) {
                                "取消"
                            } else {
                                "关闭"
                            })
                            .on_click(|_, window, _| Self::close(window)),
                    )
                    .when(matches!(self.kind, InfoWindow::Confirmation), |row| {
                        row.child(
                            Button::new("message-confirm")
                                .label(self.confirm_label.clone())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                                ),
                        )
                    }),
            )
    }

    fn about(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .p_6()
            .child(
                div()
                    .id("about-dimensional-logo")
                    .test_support()
                    .size(px(96.))
                    .child(branding::dimensional_logo().size_full()),
            )
            .child(format!("BASR {}", env!("CARGO_PKG_VERSION")))
            .child(env!("CARGO_PKG_DESCRIPTION"))
            .child("BASR 代码许可证：MIT")
            .child(
                Button::new("about-home")
                    .label("项目主页")
                    .on_click(|_, _, cx| cx.open_url(HOME_URL)),
            )
            .child(
                Button::new("about-close")
                    .label("关闭")
                    .on_click(|_, window, _| Self::close(window)),
            )
    }

    fn quit(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .child("确定退出 BASR？")
            .child(QUIT_NOTICE)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("quit-cancel")
                            .label("取消")
                            .on_click(|_, window, _| Self::close(window)),
                    )
                    .child(
                        Button::new("quit-confirm")
                            .label("退出 BASR")
                            .on_click(|_, _, cx| {
                                settings_window::flush_pending(cx);
                                cx.quit();
                            }),
                    ),
            )
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(match self.kind {
                InfoWindow::About => "about-window",
                InfoWindow::Quit => "quit-window",
                InfoWindow::Message => "message-window",
                InfoWindow::Confirmation => "confirmation-window",
            })
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .refine_style(&theme::font_style(cx))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new()
                    .child(self.title.clone())
                    .on_close_window(|_, window, _| Self::close(window)),
            )
            .child(match self.kind {
                InfoWindow::About => self.about().into_any_element(),
                InfoWindow::Quit => self.quit().into_any_element(),
                InfoWindow::Message | InfoWindow::Confirmation => {
                    self.message(cx).into_any_element()
                }
            })
    }
}
