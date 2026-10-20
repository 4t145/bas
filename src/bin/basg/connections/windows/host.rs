use std::collections::HashMap;

use bas::service::tls::ConnectionOptions;
use gpui_kit::{App, AppContext, Context, Entity, IntoElement, Render, Window};

use crate::workspace::Workspace;

/// A window owns its backend workspaces; only the selected one is presented.
/// Keeping the others alive preserves drafts and in-flight work without moving it.
pub(crate) struct Host {
    active: ConnectionOptions,
    workspaces: HashMap<ConnectionOptions, Entity<Workspace>>,
}

pub(crate) fn workspace_view(
    view: Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Host> {
    cx.new(|cx| {
        Host::install_close_handler(window, cx);
        let active = view.read(cx).connection_options().clone();
        Host {
            active: active.clone(),
            workspaces: HashMap::from([(active, view)]),
        }
    })
}

impl Host {
    pub(crate) fn active(&self) -> Entity<Workspace> {
        self.workspaces
            .get(&self.active)
            .expect("the active workspace belongs to its window")
            .clone()
    }

    pub(super) fn select(
        &mut self,
        target: ConnectionOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if self.active == target {
            return Ok(());
        }
        let previous = self.active();
        let compact = previous.read(cx).is_compact();
        let next = match self.workspaces.get(&target) {
            Some(view) => view.clone(),
            None => {
                let launch = cx
                    .try_global::<super::Launch>()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("The workspace runtime is unavailable"))?;
                cx.new(|cx| {
                    Workspace::new(
                        target.clone(),
                        launch.cwd,
                        compact,
                        &launch.runtime,
                        window,
                        cx,
                    )
                })
            }
        };
        previous.update(cx, |view, cx| view.set_presented(false, window, cx));
        self.active = target.clone();
        self.workspaces.insert(target, next.clone());
        next.update(cx, |view, cx| view.show_in_window(compact, window, cx));
        Self::install_close_handler(window, cx);
        cx.notify();
        Ok(())
    }

    fn install_close_handler(window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = owner.update(cx, |host, cx| {
                for view in host.workspaces.values() {
                    view.update(cx, |view, cx| view.set_presented(false, window, cx));
                }
            });
            true
        });
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.active()
    }
}
