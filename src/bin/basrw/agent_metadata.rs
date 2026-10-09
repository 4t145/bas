//! Agent artwork is shared by every window, independently of session connections.

use std::{collections::BTreeMap, net::SocketAddr};

use basr::{
    config::agent::{AgentSpec, Command},
    management::{Catalog, Source},
};
use gpui_kit::{App, AppContext, Context, Entity, Global, Subscription, Task};
use tokio::runtime::Handle;

#[derive(Clone)]
struct Shared(Entity<Metadata>);
impl Global for Shared {}

#[derive(Default)]
struct Server {
    icons: BTreeMap<String, String>,
    commands: BTreeMap<String, Command>,
    loaded: bool,
    requested: bool,
    queued: bool,
    changes_revision: Option<u64>,
    task: Option<Task<()>>,
}

pub(crate) struct Metadata {
    runtime: Option<Handle>,
    servers: BTreeMap<SocketAddr, Server>,
    _changes: Subscription,
}

impl Metadata {
    fn new(runtime: Option<Handle>, cx: &mut Context<Self>) -> Self {
        let changes = cx.observe_global::<crate::settings::AgentChanges>(|this, cx| {
            let changes = cx.global::<crate::settings::AgentChanges>();
            let Some(server) = changes.server else {
                return;
            };
            let revision = changes.revision;
            let state = this.servers.entry(server).or_default();
            if state.changes_revision == Some(revision) {
                return;
            }
            state.changes_revision = Some(revision);
            this.refresh(server, cx);
        });
        Self {
            runtime,
            servers: BTreeMap::new(),
            _changes: changes,
        }
    }

    /// Look up the agent that owns a thread; unknown agents keep the caller's fallback icon.
    pub(crate) fn icon(&self, server: SocketAddr, agent: &str) -> Option<String> {
        if agent == "embedded" {
            return Some(crate::assets::OUTLINE.into());
        }
        self.servers.get(&server)?.icons.get(agent).cloned()
    }

    fn ensure(&mut self, server: SocketAddr, cx: &mut Context<Self>) {
        let state = self.servers.entry(server).or_default();
        if state.loaded || state.requested {
            return;
        }
        self.refresh(server, cx);
    }

    fn refresh(&mut self, server: SocketAddr, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let state = self.servers.entry(server).or_default();
        if state.requested {
            state.queued = true;
            return;
        }
        let request =
            crate::settings::AgentCatalogService::new(server, runtime.clone()).catalog(false);
        self.begin_request(server, request, cx);
    }

    fn begin_request(
        &mut self,
        server: SocketAddr,
        request: impl Future<Output = Result<Catalog, String>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.servers.entry(server).or_default().requested = true;
        let task = cx.spawn(async move |metadata, cx| {
            let result = request.await;
            let _ = metadata.update(cx, |metadata, cx| {
                let state = metadata.servers.entry(server).or_default();
                state.requested = false;
                state.task = None;
                let queued = std::mem::take(&mut state.queued);
                if let Ok(catalog) = result {
                    metadata.apply_catalog(server, &catalog, cx);
                }
                if queued {
                    metadata.refresh(server, cx);
                }
            });
        });
        self.servers.entry(server).or_default().task = Some(task);
    }

    fn apply_catalog(&mut self, server: SocketAddr, catalog: &Catalog, cx: &mut Context<Self>) {
        let state = self.servers.entry(server).or_default();
        state.loaded = true;
        // Removed configurations can still own persisted threads; retain their last artwork.
        if merge_icons(&mut state.icons, &mut state.commands, catalog) {
            cx.notify();
        }
    }
}

/// Start one cached catalog request; subsequent windows share this entity.
pub(crate) fn init(server: SocketAddr, runtime: Handle, cx: &mut App) {
    if cx.try_global::<crate::settings::AgentChanges>().is_none() {
        cx.set_global(crate::settings::AgentChanges::default());
    }
    if cx.try_global::<Shared>().is_none() {
        let metadata = cx.new(|cx| Metadata::new(Some(runtime), cx));
        cx.set_global(Shared(metadata));
    }
    ensure(server, cx);
}

pub(crate) fn entity(cx: &App) -> Option<Entity<Metadata>> {
    cx.try_global::<Shared>().map(|shared| shared.0.clone())
}

/// Ensure metadata once when opening/reconnecting a workspace, never from a render timer.
pub(crate) fn ensure(server: SocketAddr, cx: &mut App) {
    if let Some(metadata) = entity(cx) {
        metadata.update(cx, |metadata, cx| metadata.ensure(server, cx));
    }
}

/// Reuse a settings catalog already fetched by another GUI action.
pub(crate) fn publish(server: SocketAddr, catalog: &Catalog, cx: &mut App) {
    if let Some(metadata) = entity(cx) {
        metadata.update(cx, |metadata, cx| {
            metadata.apply_catalog(server, catalog, cx)
        });
    }
}

#[cfg(all(test, feature = "gui-tests"))]
pub(crate) fn for_test(server: SocketAddr, catalog: &Catalog, cx: &mut App) {
    if cx.try_global::<crate::settings::AgentChanges>().is_none() {
        cx.set_global(crate::settings::AgentChanges::default());
    }
    if cx.try_global::<Shared>().is_none() {
        let metadata = cx.new(|cx| Metadata::new(None, cx));
        cx.set_global(Shared(metadata));
    }
    publish(server, catalog, cx);
}

fn merge_icons(
    icons: &mut BTreeMap<String, String>,
    commands: &mut BTreeMap<String, Command>,
    catalog: &Catalog,
) -> bool {
    let official: BTreeMap<_, _> = catalog
        .entries
        .iter()
        .filter(|entry| entry.source == Source::Registry && !entry.icon.is_empty())
        .map(|entry| (entry.id.as_str(), entry.icon.as_str()))
        .collect();
    let mut changed = false;
    for entry in &catalog.entries {
        let command = entry.spec.as_ref().and_then(AgentSpec::command);
        let icon = if !entry.icon.is_empty() {
            Some(entry.icon.as_str())
        } else {
            entry
                .spec
                .as_ref()
                .and_then(|spec| command_icon(spec, &official))
        };
        if let Some(icon) = icon {
            if icons.get(&entry.id).is_none_or(|previous| previous != icon) {
                icons.insert(entry.id.clone(), icon.into());
                changed = true;
            }
        } else if !official.is_empty()
            && command.is_some_and(|current| {
                commands
                    .get(&entry.id)
                    .is_some_and(|previous| previous != current)
            })
        {
            changed |= icons.remove(&entry.id).is_some();
        }
        if (icon.is_some() || !official.is_empty())
            && let Some(command) = command
        {
            commands.insert(entry.id.clone(), command.clone());
        }
    }
    changed
}

fn command_icon<'a>(spec: &AgentSpec, official: &BTreeMap<&str, &'a str>) -> Option<&'a str> {
    let command = spec.command()?;
    let filename = command.program.file_name()?.to_str()?;
    let executable = filename
        .strip_suffix(".exe")
        .or_else(|| filename.strip_suffix(".cmd"))
        .unwrap_or(filename);
    if let Some(icon) = official.get(executable) {
        return Some(icon);
    }
    // Managed JS launchers retain an exact package directory, even when their executable is Node.
    if executable == "node" {
        let script = std::path::Path::new(command.args.first()?);
        let mut parts = script.components();
        while let Some(part) = parts.next() {
            if part.as_os_str() != "node_modules" {
                continue;
            }
            let package = parts.next()?.as_os_str().to_str()?;
            let package = if package.starts_with('@') {
                parts.next()?.as_os_str().to_str()?
            } else {
                package
            };
            if let Some(icon) = official.get(package) {
                return Some(icon);
            }
        }
    }
    None
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
