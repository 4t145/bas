//! Application-only windows; these never send commands to bas.

use gds_uim::el;
use gpui_kit::{
    App, AppContext, Bounds, ClickEvent, Context, Global, KeyBinding, Render, TestSupportExt,
    Window, WindowBounds, WindowDecorations, WindowHandle, WindowOptions, actions,
    component::{ActiveTheme, Root, StyledExt, TitleBar},
    div,
    prelude::*,
    px, size,
};

use crate::{branding, settings, theme};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

pub(crate) const RELEASES_URL: &str = "https://github.com/4t145/bas/releases";
const HOME_URL: &str = "https://github.com/4t145/bas";
const WIDTH: f32 = 480.;
const HEIGHT: f32 = 420.;

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
    cx.on_action(on_open_settings);
    cx.on_action(on_quit);
}

fn on_open_settings(_: &OpenSettings, cx: &mut App) {
    if let Err(error) = settings::window::open(cx) {
        tracing::error!(%error, "cannot open settings");
    }
}

fn on_quit(_: &Quit, cx: &mut App) {
    if let Err(error) = open_quit(cx) {
        tracing::error!(%error, "cannot open quit confirmation");
    }
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

// [!todo] open confirmations and open messages doesn't actiually work on niri/wayland

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
            // [!todo] here we have product name `BasG`, witch should be a static value and easy to replace with
            // other name
            Self::About => crate::i18n::tr!("app-about"),
            Self::Quit => crate::i18n::tr!("app-quit-title"),
            Self::Message => "BasG",
            Self::Confirmation => crate::i18n::tr!("app-confirm"),
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
            app_id: Some("org.bas.Gui".into()),
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

    fn on_close(_: &ClickEvent, window: &mut Window, _: &mut App) {
        Self::close(window);
    }

    fn on_confirm(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx);
    }

    fn on_home(_: &ClickEvent, _: &mut Window, cx: &mut App) {
        cx.open_url(HOME_URL);
    }

    fn on_quit_confirm(_: &ClickEvent, _: &mut Window, cx: &mut App) {
        settings::window::flush_pending(cx);
        cx.quit();
    }

    fn message(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let confirmation = matches!(self.kind, InfoWindow::Confirmation);
        let actions = el! {
            #div {
                flex; flex_shrink_0; justify_end; gap_3;
            } [
                #crate::ui::action_button("message-close") {
                    label: $if confirmation {
                        crate::i18n::tr!("action-cancel")
                    } else {
                        crate::i18n::tr!("action-close")
                    };
                    on_click: Self::on_close;
                },
                ..$confirmation.then(|| el! {
                    #crate::ui::action_button("message-confirm") {
                        label: $self.confirm_label.clone();
                        on_click: $cx.listener(Self::on_confirm);
                    }
                }),
            ]
        };
        el! {
            #div {
                flex; flex_col; flex_1; min_h_0; gap_6; p_6;
            } [
                #div {
                    id: "application-message-body";
                    test_support;
                    role: gpui_kit::Role::Label;
                    aria_label: $self.message.clone();
                    flex_1; min_h_0; overflow_y_scroll; whitespace_normal;
                } [$crate::ui::tooltip::wrap_tokens(&self.message)],
                $actions,
            ]
        }
    }

    fn about(&self) -> impl IntoElement {
        el! {
            #div {
                flex; flex_col; items_center; gap_3; p_6;
            } [
                #div {
                    id: "about-dimensional-logo";
                    test_support;
                    size: 96.0.px;
                } [#branding::dimensional_logo { size_full; }],
                $format!("BasG {}", env!("CARGO_PKG_VERSION")),
                $crate::i18n::tr!("app-description"),
                $crate::i18n::tr!("app-license"),
                #crate::ui::action_button("about-home") {
                    label: $crate::i18n::tr!("app-home");
                    on_click: Self::on_home;
                },
                #crate::ui::action_button("about-close") {
                    label: $crate::i18n::tr!("action-close");
                    on_click: Self::on_close;
                },
            ]
        }
    }

    fn quit(&self) -> impl IntoElement {
        el! {
            #div {
                flex; flex_col; gap_6; p_6;
            } [
                $crate::i18n::tr!("app-quit-question"),
                $crate::i18n::tr!("app-quit-notice"),
                #div {
                    flex; justify_end; gap_3;
                } [
                    #crate::ui::action_button("quit-cancel") {
                        label: $crate::i18n::tr!("action-cancel");
                        on_click: Self::on_close;
                    },
                    #crate::ui::action_button("quit-confirm") {
                        label: $crate::i18n::tr!("app-quit");
                        on_click: Self::on_quit_confirm;
                    },
                ],
            ]
        }
    }
}

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if matches!(self.kind, InfoWindow::About | InfoWindow::Quit) {
            self.title = self.kind.title().into();
            window.set_window_title(&self.title);
        }
        let id = match self.kind {
            InfoWindow::About => "about-window",
            InfoWindow::Quit => "quit-window",
            InfoWindow::Message => "message-window",
            InfoWindow::Confirmation => "confirmation-window",
        };
        let content = match self.kind {
            InfoWindow::About => self.about().into_any_element(),
            InfoWindow::Quit => self.quit().into_any_element(),
            InfoWindow::Message | InfoWindow::Confirmation => self.message(cx).into_any_element(),
        };
        el! {
            #div {
                id: id;
                test_support;
                size_full; flex; flex_col;
                refine_style: $&theme::font_style(cx);
                bg: $cx.theme().background;
                text_color: $cx.theme().foreground;
            } [
                TitleBar {
                    on_close_window: Self::on_close;
                } [$self.title.clone()],
                $content,
            ]
        }
    }
}
