//! Installation is a native surface, not a settings section.
use super::{Agents, Editor, Page};
use crate::{theme, ui::window_controls::TitleBar};
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Global, Render, Subscription, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, component::Root, div, prelude::*, px,
    size,
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

struct OpenWindow(WindowHandle<Root>);
impl Global for OpenWindow {}

const WIDTH: f32 = 900.;
const HEIGHT: f32 = 720.;
const MIN_WIDTH: f32 = 640.;
const MIN_HEIGHT: f32 = 480.;

gpui_kit::actions!(agent_installation_window, [CloseWindow]);

/// # Errors
/// Returns a platform error if the onboarding window cannot be created.
pub(in crate::settings) fn open_catalog(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    open_inner(None, cx)
}

/// # Errors
/// Returns a platform error if the onboarding window cannot be created.
pub(in crate::settings) fn open(
    agents: Entity<Agents>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Root>> {
    open_inner(Some(agents), cx)
}

/// # Errors
/// Returns a platform error when native window creation fails.
fn open_inner(agents: Option<Entity<Agents>>, cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    if let Some(handle) = current(cx)
        && cx.windows().contains(&handle.into())
    {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        });
        return Ok(handle);
    }
    cx.bind_keys([gpui_kit::KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-w"
        },
        CloseWindow,
        Some("BasAgentInstallation"),
    )]);
    let handle = cx.open_window(
        WindowOptions {
            kind: WindowKind::Floating,
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(WIDTH), px(HEIGHT)),
                cx,
            ))),
            window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
            window_background: cx.global::<theme::Appearance>().background,
            app_id: Some("org.bas.Gui".into()),
            ..TitleBar::window_options()
        },
        |window, cx| {
            let agents = agents.unwrap_or_else(|| super::shared(window, cx));
            let host = cx.new(|cx| {
                let observation = cx.observe(&agents, |_, _, cx| cx.notify());
                let weak = agents.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    let _ = weak.update(cx, |agents, cx| {
                        cancel_draft(agents, cx);
                    });
                    true
                });
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                Host {
                    agents,
                    focus,
                    _observation: observation,
                }
            });
            cx.new(|cx| theme::window_root(host, window, cx))
        },
    )?;
    cx.set_global(OpenWindow(handle));
    Ok(handle)
}

pub(in crate::settings) fn current(cx: &App) -> Option<WindowHandle<Root>> {
    cx.try_global::<OpenWindow>().map(|state| state.0)
}

struct Host {
    agents: Entity<Agents>,
    focus: FocusHandle,
    _observation: Subscription,
}

fn cancel_draft(agents: &mut Agents, cx: &mut Context<Agents>) {
    if matches!(agents.editor, Editor::Add(_)) {
        agents.editor = Editor::Idle;
        agents.message.clear();
        cx.notify();
    }
}

impl Host {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agents.update(cx, cancel_draft);
        window.remove_window();
    }

    fn on_close_click(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close(window, cx);
    }

    fn on_close_action(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }
}

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(crate::i18n::tr!("agent-install-action"));
        let content = self.agents.update(cx, |agents, cx| {
            agents.render_page(Page::Onboarding, window, cx)
        });
        gds_uim::el! {
            #div {
                size_full; flex; flex_col;
                key_context: "BasAgentInstallation";
                track_focus: $&self.focus;
                on_action: $cx.listener(Self::on_close_action);
            } [
                TitleBar { on_close_window: $cx.listener(Self::on_close_click); } [$crate::i18n::tr!("agent-install-action")],
                #div { flex_1; min_h_0; } [$content],
            ]
        }
    }
}
