//! Daemon-owned ACP discovery and installation. The GUI never executes registry commands.

use super::{Agents, item};
use crate::agents::catalog::Updates;
use crate::{
    i18n::tr,
    settings,
    ui::{self, Button},
};
use bas::management::{Catalog, Entry, Source, Status};
use gds_uim::el;
use gpui_kit::{
    App, Context, Hsla, IntoElement, ParentElement, Styled, Task, TestSupportExt, div, prelude::*,
};
use gpui_kit::{
    assets::IconName,
    component::{button::*, input::*, *},
};

const REGISTRY_URL: &str = "https://github.com/agentclientprotocol/registry";

/// Refreshes retain the last catalog; configuration RPC capability grants edits.
pub(super) struct Registry {
    pub(super) catalog: Option<Catalog>,
    access: Access,
    load: Load,
}

enum Access {
    Standalone,
    AwaitingProfile,
    Editable,
    OtherProfile,
    Unavailable,
}

enum Load {
    Idle,
    Loading { _task: Task<()> },
}

impl Registry {
    pub(super) fn awaiting_profile(&self) -> bool {
        matches!(self.access, Access::AwaitingProfile)
    }

    pub(super) fn new(has_service: bool) -> Self {
        Self {
            catalog: None,
            access: if has_service {
                Access::AwaitingProfile
            } else {
                Access::Standalone
            },
            load: Load::Idle,
        }
    }

    pub(super) fn editable(&self) -> bool {
        matches!(self.access, Access::Standalone | Access::Editable)
    }

    fn loading(&self) -> bool {
        matches!(self.load, Load::Loading { .. })
    }
}

fn offered(entry: &Entry) -> bool {
    entry.source == Source::Registry
        && entry.spec.is_none()
        && entry.installed_version.is_none()
        && !matches!(entry.status, Status::Installed | Status::Missing)
}

pub(super) struct Palette {
    pub background: Hsla,
    card: Hsla,
    border: Hsla,
    accent: Hsla,
}

impl Palette {
    pub(super) fn current(cx: &App) -> Self {
        Self {
            background: cx.theme().background,
            card: cx.theme().muted,
            border: cx.theme().border,
            accent: cx.theme().primary,
        }
    }
}

pub(super) struct Operation {
    message: String,
    pub(super) busy: bool,
    _task: Option<Task<()>>,
}

fn matches_query(entry: &Entry, query: &str) -> bool {
    let text = format!(
        "{} {} {} {}",
        entry.id, entry.name, entry.description, entry.repository
    )
    .to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

fn status_label(entry: &Entry) -> &'static str {
    match entry.status {
        Status::NotInstalled => crate::i18n::tr!("agent-not-installed"),
        Status::Installed => crate::i18n::tr!("agent-installed"),
        Status::Missing => crate::i18n::tr!("agent-program-unavailable"),
        Status::Unsupported => crate::i18n::tr!("agent-platform-unsupported"),
    }
}

impl Agents {
    pub(super) fn notify_agent_change(&self, ready: Option<String>, cx: &mut Context<Self>) {
        let Some(service) = self.catalog_service.clone() else {
            return;
        };
        if cx.try_global::<crate::agents::Changes>().is_none() {
            return;
        }
        cx.update_global::<crate::agents::Changes, _>(|changes, _| {
            changes.server = Some(service.address());
            changes.ready = ready;
            changes.select = None;
            changes.revision += 1;
        });
    }

    pub(super) fn begin_chat(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(service) = self.catalog_service.clone() else {
            return;
        };
        if let Err(error) = crate::connections::open_for(service.address(), cx) {
            self.message = error.to_string();
            cx.notify();
            return;
        }
        cx.update_global::<crate::agents::Changes, _>(|changes, _| {
            changes.server = Some(service.address());
            changes.ready = Some(id.clone());
            changes.select = Some(id);
            changes.revision += 1;
        });
    }

    pub(super) fn refresh_registry(&mut self, refresh: bool, cx: &mut Context<Self>) {
        let Some(service) = self.catalog_service.clone() else {
            return;
        };
        if self.registry.loading() {
            return;
        }
        let request = service.catalog(refresh);
        self.registry.load = Load::Loading {
            _task: cx.spawn(async move |page, cx| {
                let result = request.await;
                let _ = page.update(cx, |page, cx| {
                    page.registry.load = Load::Idle;
                    match result {
                        Ok(catalog) => page.apply_catalog(catalog, cx),
                        Err(error) => {
                            page.registry.access = if page.configuration.is_some() {
                                Access::Editable
                            } else {
                                Access::Unavailable
                            };
                            page.message =
                                crate::i18n::tr!("catalog-load-error", error = (error).to_string());
                        }
                    }
                    cx.notify();
                });
            }),
        };
        cx.notify();
    }

    pub(super) fn apply_catalog(&mut self, catalog: Catalog, cx: &mut Context<Self>) {
        if let Some(service) = &self.catalog_service {
            crate::agents::metadata::publish(service.address(), &catalog, cx);
        }
        self.registry.access = if self.configuration.is_some() {
            Access::Editable
        } else {
            Access::OtherProfile
        };
        self.registry.catalog = Some(catalog);
        {
            let specs: Vec<_> = self
                .registry
                .catalog
                .as_ref()
                .into_iter()
                .flat_map(|catalog| &catalog.entries)
                .filter_map(|entry| entry.spec.clone())
                .collect();
            self.checks
                .retain(|id, _| specs.iter().any(|spec| spec.id == *id));
            for spec in specs {
                if self
                    .checks
                    .get(&spec.id)
                    .is_none_or(|check| check.spec != spec)
                {
                    self.start_check(spec, false, cx);
                }
            }
        }
        if let Some(id) = self.pending_reveal.take()
            && self.registry.editable()
        {
            self.reveal(&id, cx);
        }
    }

    pub(super) fn managed_allowed(&self, cx: &App) -> bool {
        self.registry.editable()
            && self.configuration_task.is_none()
            && !self.dirty(cx)
            && self
                .registry
                .catalog
                .as_ref()
                .is_some_and(|catalog| catalog.can_install)
    }

    fn install(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.managed_allowed(cx) || self.operations.values().any(|operation| operation.busy) {
            return;
        }
        let Some(service) = self.catalog_service.clone() else {
            return;
        };
        self.start_install_updates(id.clone(), service.install(id), cx);
    }

    fn start_install_updates(&mut self, id: String, mut updates: Updates, cx: &mut Context<Self>) {
        let key = id.clone();
        let task = cx.spawn(async move |page, cx| {
            while let Some(update) = updates.next().await {
                let finished = match &update {
                    Ok(operation) => operation.complete,
                    Err(_) => true,
                };
                let _ = page.update(cx, |page, cx| {
                    let message = match &update {
                        Ok(operation) if operation.complete => String::new(),
                        Ok(operation) => operation.message.clone(),
                        Err(error) => {
                            crate::i18n::tr!("agent-install-error", error = (error).to_string())
                        }
                    };
                    if let Some(operation) = page.operations.get_mut(&key) {
                        operation.message = message;
                        operation.busy = !finished;
                    }
                    if let Ok(operation) = update
                        && operation.complete
                    {
                        if !page.dirty(cx) && page.catalog_service.is_none() {
                            match page.store.load() {
                                Ok(document) => page.accept_document(document),
                                Err(error) => {
                                    page.message = crate::i18n::tr!(
                                        "settings-load-error",
                                        error = error.to_string()
                                    )
                                }
                            }
                        }
                        page.notify_agent_change(None, cx);
                        if let Some(spec) = operation.spec {
                            page.saved = Some(spec.clone());
                            page.start_check(spec, true, cx);
                        }
                    }
                    if finished {
                        page.fetch_configuration(cx);
                    }
                    cx.notify();
                });
                if finished {
                    return;
                }
            }
            let _ = page.update(cx, |page, cx| {
                if let Some(operation) = page.operations.get_mut(&key)
                    && operation.busy
                {
                    operation.busy = false;
                    operation.message = crate::i18n::tr!("agent-install-disconnected").into();
                }
                page.refresh_registry(false, cx);
                cx.notify();
            });
        });
        self.operations.insert(
            id,
            Operation {
                message: crate::i18n::tr!("agent-install-preparing").into(),
                busy: true,
                _task: Some(task),
            },
        );
        cx.notify();
    }

    fn cancel_install(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(operation) = self.operations.get_mut(id) {
            operation._task.take();
            operation.busy = false;
            operation.message = crate::i18n::tr!("agent-install-cancelled").into();
        }
        self.refresh_registry(false, cx);
        cx.notify();
    }

    pub(super) fn remove_managed(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.registry.editable()
            || self.configuration_task.is_some()
            || self.dirty(cx)
            || self.operations.values().any(|operation| operation.busy)
        {
            return;
        }
        let Some(service) = self.catalog_service.clone() else {
            return;
        };
        let request = service.remove(id.clone());
        let key = id.clone();
        let task = cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                if let Some(operation) = page.operations.get_mut(&key) {
                    operation.busy = false;
                    operation.message = match result {
                        Ok(()) => String::new(),
                        Err(error) => {
                            crate::i18n::tr!("agent-remove-error", error = (error).to_string())
                        }
                    };
                }
                page.notify_agent_change(None, cx);
                page.fetch_configuration(cx);
                cx.notify();
            });
        });
        self.operations.insert(
            id,
            Operation {
                message: crate::i18n::tr!("agent-removing").into(),
                busy: true,
                _task: Some(task),
            },
        );
        cx.notify();
    }

    pub(super) fn registry_actions(
        &self,
        entry: &Entry,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let busy = self
            .operations
            .get(&entry.id)
            .is_some_and(|operation| operation.busy);
        let any_busy = self.operations.values().any(|operation| operation.busy);
        let update = entry
            .installed_version
            .as_ref()
            .is_some_and(|version| !entry.version.is_empty() && version != &entry.version);
        let action = if busy {
            let id = entry.id.clone();
            Some(el! {
                (format!("agents-install-cancel-{}", entry.id), IconName::X, tr!("agent-cancel-action")).item::action {
                    on_click: $cx.listener(move |this, _, _, cx| this.cancel_install(&id, cx));
                }
            }.into_any_element())
        } else if entry.source == Source::Registry
            && (matches!(entry.status, Status::NotInstalled | Status::Missing) || update)
        {
            let id = entry.id.clone();
            let disabled =
                !self.managed_allowed(cx) || any_busy || entry.status == Status::Unsupported;
            Some(el! {
                (format!("agents-install-{}", entry.id),
                    @if (update) IconName::CircleArrowUp @else IconName::Download @end,
                    @if (update) tr!("agent-update-action") @else tr!("agent-install-action") @end
                ).item::action {
                    disabled: disabled;
                    on_click: $cx.listener(move |this, _, _, cx| this.install(id.clone(), cx));
                }
            }.into_any_element())
        } else if entry.spec.is_some() {
            None
        } else {
            Some(el! {
                (format!("agents-registry-unavailable-{}", entry.id), IconName::CircleAlert, (entry).status_label).item::action {
                    disabled: true;
                }
            }.into_any_element())
        };
        el! {
            .div { flex; items_center; gap_1; flex_shrink_0; } [
                @if (let Some(spec) = entry.spec.as_ref().filter(|spec| !busy && spec.kind == bas::config::agent::Kind::Acp))
                    $self.check_button(format!("agents-registry-check-{}", entry.id), spec, cx),
                @end,
                ..action,
                @if (let Some(spec) = entry.spec.as_ref().filter(|_| !busy))
                    $self.account_actions(spec, cx),
                @end,
                @if (!busy && (entry.installed_version.is_some() || matches!(entry.status, Status::Installed | Status::Missing)))
                    $self.remove_button(&entry.id, cx),
                @end,
            ]
        }.into_any_element()
    }

    pub(super) fn item_feedback(&self, entry: &Entry) -> Option<String> {
        let status = entry.spec.as_ref().and_then(|spec| self.status(spec));
        let has_authentication = entry
            .spec
            .as_ref()
            .is_some_and(|spec| self.authentications.contains_key(&spec.id));
        let operation = self.operations.get(&entry.id);
        if let Some(message) = entry
            .spec
            .as_ref()
            .and_then(|spec| self.logout_message(spec))
        {
            return Some(message);
        }
        operation
            .filter(|operation| operation.busy)
            .map(|operation| operation.message.clone())
            .or_else(|| match status.filter(|_| !has_authentication) {
                Some(
                    crate::agents::Status::Failed(message)
                    | crate::agents::Status::Missing(message),
                ) => Some(message.clone()),
                _ => None,
            })
            .or_else(|| {
                operation
                    .filter(|operation| !operation.message.is_empty())
                    .map(|operation| operation.message.clone())
            })
            .or_else(|| {
                (!entry.message.is_empty()
                    && !(entry.status == Status::Installed && entry.source == Source::Registry))
                    .then(|| entry.message.clone())
            })
    }

    fn registry_card(&self, entry: &Entry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let card = item::Item {
            entry: entry.clone(),
            status: self.item_status(entry),
            status_hint: status_label(entry).into(),
            feedback: self.item_feedback(entry),
            actions: self.registry_actions(entry, cx),
            content: None,
        };
        el! {
            .div {
                # format!("agents-registry-{}", entry.id); test_support;
                w_full; min_w_0;
            } [card]
        }
        .into_any_element()
    }

    pub(super) fn registry_header(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = Palette::current(cx);
        let title = el! {
            .div { flex; items_center; justify_between; gap_2; } [
                .div { flex; items_center; gap_3; } [
                    .div { text_size: ui::TITLE; } [tr!("catalog-title")],
                    ("agents-registry-learn-more").Button::new {
                        ghost; xsmall; label: tr!("agent-source-registry");
                        text_color: $palette.accent; icon: IconName::ExternalLink;
                        on_click: (cx, Self::on_registry_learn_more).Context::listener;
                    },
                ],
                ("agents-add-custom").Button::new {
                    small; icon: IconName::Plus; label: tr!("agent-add-custom");
                    disabled: $(self.document.is_none() || !self.registry.editable() || self.dirty(cx));
                    on_click: (cx, Self::on_add_custom).Context::listener;
                },
            ]
        };
        let toolbar = el! {
            .div { flex; items_center; justify_end; gap_3; } [
                ("agents-registry-refresh").Button::new {
                    ghost; xsmall; icon: IconName::RefreshCw;
                    label: @if (self.registry.loading())
                        tr!("catalog-refreshing")
                    @else
                        tr!("catalog-refresh")
                    @end;
                    disabled: $self.registry.loading();
                    on_click: (cx, Self::on_refresh_registry).Context::listener;
                },
            ]
        };
        let search_icon = el! { (IconName::Search).Icon::new { small; } };
        el! {
            .div { flex; flex_col; gap_3; } [
                title,
                .div {
                    # "agents-registry-search"; test_support; w_full; p_1; bg: $palette.card;
                    border_1; border_color: $palette.border; rounded_md;
                } [
                    $ui::input::placeholder(&self.registry_search,
                        Input::new(&self.registry_search).small().appearance(false).cleanable(true)
                            .prefix(search_icon).aria_label(tr!("agent-search"))),
                ],
                toolbar,
            ]
        }
        .into_any_element()
    }

    fn on_registry_learn_more(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        cx.open_url(REGISTRY_URL);
    }

    fn on_refresh_registry(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_registry(true, cx);
    }

    pub(super) fn registry_content(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let query = format!("{} {}", self.query, self.registry_search.read(cx).value());
        let entries: Vec<_> =
            self.registry
                .catalog
                .as_ref()
                .map(|catalog| {
                    catalog
                        .entries
                        .iter()
                        .filter(|entry| {
                            offered(entry)
                                && !(self.registry.editable()
                                    && self.document.as_ref().is_some_and(|document| {
                                        document.settings.agents.iter().any(|registration| {
                                            registration.id().as_str() == entry.id
                                        })
                                    }))
                                && matches_query(entry, &query)
                        })
                        .collect()
                })
                .unwrap_or_default();
        let warning = self
            .registry
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.warning.as_ref());
        el! {
            .div { # "agents-registry"; test_support; w_full; min_w_0; flex; flex_col; gap_2; } [
                @if (let Some(warning) = warning)
                    ("agents-registry-warning", ~warning, $cx.theme().muted_foreground).settings::feedback,
                @end,
                @if (entries.is_empty())
                    .div { py_4; } [
                        ("agents-registry-status",
                            @if (self.registry.loading())
                                tr!("catalog-loading")
                            @else if (self.registry.catalog.is_none())
                                tr!("catalog-connect")
                            @else
                                tr!("catalog-no-results")
                            @end,
                            $cx.theme().muted_foreground
                        ).settings::feedback,
                    ],
                @end,
                ..$entries.into_iter().map(|entry| self.registry_card(entry, cx)),
            ]
        }
        .into_any_element()
    }
}

#[cfg(test)]
mod tests;
