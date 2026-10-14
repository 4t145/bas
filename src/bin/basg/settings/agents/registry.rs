//! Daemon-owned ACP discovery and installation. The GUI never executes registry commands.

use super::Agents;
use crate::agents::catalog::{Service, Updates, settings_match};
use crate::ui::Button;
use bas::management::{Catalog, Entry, Source, Status};
use gpui_kit::{
    App, Context, FontWeight, Hsla, IntoElement, ParentElement, Styled, Task, TestSupportExt, div,
    prelude::*, px,
};
use gpui_kit::{
    assets::IconName,
    component::{button::*, input::*, *},
};

const REGISTRY_URL: &str = "https://github.com/agentclientprotocol/registry";
const ICON_SIZE: f32 = 22.;

/// Refreshes retain the last catalog; only a verified local profile grants daemon-backed edits.
pub(super) struct Registry {
    pub(super) catalog: Option<Catalog>,
    access: Access,
    load: Load,
}

enum Access {
    Standalone,
    AwaitingProfile,
    LocalProfile,
    OtherProfile,
    Remote,
    Unavailable,
}

enum Load {
    Idle,
    Loading { _task: Task<()> },
}

impl Registry {
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
        matches!(self.access, Access::Standalone | Access::LocalProfile)
    }

    fn loading(&self) -> bool {
        matches!(self.load, Load::Loading { .. })
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Filter {
    #[default]
    All,
    Installed,
    NotInstalled,
}

impl Filter {
    fn matches(self, entry: &Entry) -> bool {
        let installed = entry.spec.is_some()
            || entry.installed_version.is_some()
            || matches!(entry.status, Status::Installed | Status::Missing);
        match self {
            Self::All => true,
            Self::Installed => installed,
            Self::NotInstalled => !installed,
        }
    }
}

pub(super) struct Palette {
    pub background: Hsla,
    card: Hsla,
    border: Hsla,
    accent: Hsla,
    action_fill: Hsla,
    action_foreground: Hsla,
    muted: Hsla,
}

impl Palette {
    pub(super) fn current(cx: &App) -> Self {
        Self {
            background: cx.theme().background,
            card: cx.theme().muted,
            border: cx.theme().border,
            accent: cx.theme().primary,
            action_fill: cx.theme().primary,
            action_foreground: cx.theme().primary_foreground,
            muted: cx.theme().muted_foreground,
        }
    }
}

pub(super) struct Operation {
    message: String,
    busy: bool,
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
        let Some(service) = cx.try_global::<Service>().cloned() else {
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

    pub(super) fn begin_chat(&self, id: String, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<Service>().cloned() else {
            return;
        };
        if cx.try_global::<crate::agents::Changes>().is_none() {
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
        let Some(service) = cx.try_global::<Service>().cloned() else {
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
                        Ok(catalog) => page.apply_catalog(catalog, service.local(), cx),
                        Err(error) => {
                            page.registry.access = Access::Unavailable;
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

    fn apply_catalog(&mut self, catalog: Catalog, local: bool, cx: &mut Context<Self>) {
        if let Some(service) = cx.try_global::<Service>().cloned() {
            crate::agents::metadata::publish(service.address(), &catalog, cx);
        }
        self.registry.access = if !local {
            Access::Remote
        } else if settings_match(&catalog, self.store.path()) {
            Access::LocalProfile
        } else {
            Access::OtherProfile
        };
        self.registry.catalog = Some(catalog);
        if self.registry.editable() {
            if !self.dirty(cx) {
                match self.store.load() {
                    Ok(document) => self.accept_document(document),
                    Err(error) => {
                        self.message =
                            crate::i18n::tr!("settings-load-error", error = (error).to_string())
                    }
                }
            }
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
        } else {
            self.message = if local {
                crate::i18n::tr!("catalog-config-mismatch").into()
            } else {
                crate::i18n::tr!("catalog-remote").into()
            };
        }
        if let Some(id) = self.pending_reveal.take()
            && self.registry.editable()
        {
            self.reveal(&id, cx);
        }
    }

    fn managed_allowed(&self, cx: &App) -> bool {
        self.registry.editable()
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
        let Some(service) = cx.try_global::<Service>().cloned() else {
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
                        Ok(operation) if operation.complete => {
                            crate::i18n::tr!("agent-install-done").into()
                        }
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
                        if !page.dirty(cx) {
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
                            page.start_check(spec, true, cx);
                        }
                    }
                    if finished {
                        page.refresh_registry(false, cx);
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

    fn remove_managed(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.managed_allowed(cx) || self.operations.values().any(|operation| operation.busy) {
            return;
        }
        let Some(service) = cx.try_global::<Service>().cloned() else {
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
                        Ok(()) => crate::i18n::tr!("agent-removed").into(),
                        Err(error) => {
                            crate::i18n::tr!("agent-remove-error", error = (error).to_string())
                        }
                    };
                }
                page.notify_agent_change(None, cx);
                page.refresh_registry(false, cx);
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

    fn registry_icon(entry: &Entry, palette: &Palette) -> gpui_kit::AnyElement {
        gds_uim::el! {
            #div {
                size: $px(32.); flex_shrink_0; flex; items_center; justify_center;
                rounded_md; text_color: $palette.accent;
            } [
                crate::agents::icon::AgentIcon($format!("agents-icon-{}", entry.id), $entry.icon.clone()) {
                    size: $px(ICON_SIZE); text_color: $palette.accent;
                },
            ]
        }.into_any_element()
    }

    fn registry_links(&self, entry: &Entry, palette: &Palette) -> gpui_kit::AnyElement {
        let repository = entry.repository.clone();
        let license = entry.license_url.clone();
        gds_uim::el! {
            #div {
                flex; items_center; flex_shrink_0; gap_2;
                text_size: $crate::ui::SMALL; text_color: $palette.muted;
            } [
                #div [ $format!("ID: {}", entry.id) ],
                ..$bool::then(!repository.is_empty(), || gds_uim::el! {
                    Button($format!("agents-repository-{}", entry.id)) {
                        ghost; xsmall; icon: $IconName::Github;
                        tooltip: $crate::i18n::tr!("agent-repository");
                        text_color: $palette.muted;
                        on_click: $move |_, _, cx| cx.open_url(&repository);
                    }
                }),
                ..$bool::then(!entry.license.is_empty(), || gds_uim::el! {
                    Button($format!("agents-license-{}", entry.id)) {
                        ghost; xsmall; icon: $IconName::Scale;
                        tooltip: $crate::i18n::tr!(
                            "agent-license",
                            license = (entry.license).to_string()
                        );
                        text_color: $palette.muted; disabled: $license.is_empty();
                        on_click: $move |_, _, cx| cx.open_url(&license);
                    }
                }),
            ]
        }
        .into_any_element()
    }

    fn registry_actions(&self, entry: &Entry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let status = entry.spec.as_ref().and_then(|spec| self.status(spec));
        let busy = self
            .operations
            .get(&entry.id)
            .is_some_and(|operation| operation.busy);
        let any_busy = self.operations.values().any(|operation| operation.busy);
        let update = entry
            .installed_version
            .as_ref()
            .is_some_and(|version| !entry.version.is_empty() && version != &entry.version);
        let palette = Palette::current(cx);
        let action = if busy {
            let id = entry.id.clone();
            gds_uim::el! {
                $super::button(format!("agents-install-cancel-{}", entry.id)) {
                    label: $crate::i18n::tr!("action-cancel");
                    on_click: $cx.listener(move |this, _, _, cx| this.cancel_install(&id, cx));
                }
            }
        } else if entry.source == Source::Registry
            && (matches!(entry.status, Status::NotInstalled | Status::Missing) || update)
        {
            let id = entry.id.clone();
            let disabled =
                !self.managed_allowed(cx) || any_busy || entry.status == Status::Unsupported;
            gds_uim::el! {
                $super::button(format!("agents-install-{}", entry.id)) {
                    primary;
                    label: $(if update {
                        crate::i18n::tr!("action-update")
                    } else {
                        crate::i18n::tr!("action-install")
                    });
                    icon: $IconName::Download;
                    @$(!disabled) => {
                        bg: $palette.action_fill; text_color: $palette.action_foreground;
                    };
                    disabled: $disabled;
                    on_click: $cx.listener(move |this, _, _, cx| this.install(id.clone(), cx));
                }
            }
        } else if let Some(spec) = entry.spec.clone() {
            let id = entry.id.clone();
            let embedded = spec.kind == bas::config::agent::Kind::Embedded;
            gds_uim::el! {
                $super::button(format!("agents-registry-check-{}", entry.id)) {
                    label: $(if embedded {
                        crate::i18n::tr!("agent-start-chat")
                    } else {
                        status.map_or(crate::i18n::tr!("agent-check"), super::status::check_label)
                    });
                    tooltip: $(if embedded {
                        crate::i18n::tr!("agent-start-chat")
                    } else {
                        crate::i18n::tr!("agent-check-help")
                    });
                    text_color: $palette.accent;
                    disabled: $(!self.registry.editable() || self.dirty(cx)
                        || status.is_some_and(crate::agents::Status::busy));
                    on_click: $cx.listener(move |this, _, _, cx| this.check_registry_agent(&id, &spec, cx));
                }
            }
        } else {
            gds_uim::el! {
                $super::button(format!("agents-registry-unavailable-{}", entry.id)) {
                    label: $status_label(entry); disabled: true;
                }
            }
        };
        let remove = entry.id.clone();
        let can_chat = entry
            .spec
            .as_ref()
            .is_some_and(|spec| spec.kind != bas::config::agent::Kind::Embedded)
            && status.is_some_and(crate::agents::Status::connected)
            && !busy;
        gds_uim::el! {
            #div { flex; items_center; gap_2; flex_shrink_0; } [
                $action,
                ..$can_chat.then(|| {
                    let id = entry.id.clone();
                    gds_uim::el! {
                        Button($format!("agents-registry-start-chat-{}", entry.id)) {
                            ghost; xsmall; label: $crate::i18n::tr!("agent-start-chat");
                            text_color: $palette.accent;
                            disabled: $(!self.registry.editable() || self.dirty(cx));
                            on_click: $cx.listener(move |this, _, _, cx| this.begin_chat(id.clone(), cx));
                        }
                    }
                }),
                ..$bool::then(entry.installed_version.is_some() && !busy, || gds_uim::el! {
                    Button($format!("agents-remove-{}", entry.id)) {
                        ghost; xsmall; label: $crate::i18n::tr!("action-remove");
                        disabled: $(!self.managed_allowed(cx) || any_busy);
                        on_click: $cx.listener(move |this, _, _, cx| this.remove_managed(remove.clone(), cx));
                    }
                }),
            ]
        }.into_any_element()
    }

    fn check_registry_agent(
        &mut self,
        id: &str,
        spec: &bas::config::AgentSpec,
        cx: &mut Context<Self>,
    ) {
        if spec.kind == bas::config::agent::Kind::Embedded {
            self.begin_chat(id.to_owned(), cx);
        } else {
            self.start_check(spec.clone(), true, cx);
        }
    }

    fn registry_card(&self, entry: &Entry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = Palette::current(cx);
        let status = entry.spec.as_ref().and_then(|spec| self.status(spec));
        let has_authentication = entry
            .spec
            .as_ref()
            .is_some_and(|spec| self.authentications.contains_key(&spec.id));
        let status_label = if entry.source == Source::Embedded {
            crate::i18n::tr!("agent-builtin-ready")
        } else {
            status.map_or(
                crate::i18n::tr!("agent-installed-unchecked"),
                super::status::label,
            )
        };
        let check_detail = entry.spec.as_ref().map(|spec| self.check_detail(spec));
        let operation = self.operations.get(&entry.id);
        let detail = operation
            .filter(|operation| operation.busy)
            .map(|operation| operation.message.clone())
            .or_else(|| match status.filter(|_| !has_authentication) {
                Some(
                    crate::agents::Status::Failed(message)
                    | crate::agents::Status::Missing(message),
                ) => Some(message.clone()),
                _ => None,
            })
            .or_else(|| operation.map(|operation| operation.message.clone()))
            .or_else(|| {
                (!entry.message.is_empty()).then(|| {
                    if entry.status == Status::Installed && entry.source == Source::Registry {
                        crate::i18n::tr!("agent-install-done").into()
                    } else {
                        entry.message.clone()
                    }
                })
            });
        let status_hint = (!has_authentication
            && (status.is_some() || entry.status == Status::Installed))
            .then(|| {
                crate::ui::tooltip::bounded(
                    format!("agents-registry-status-hint-{}", entry.id),
                    gds_uim::el! {
                        #div {
                            id: $format!("agents-registry-status-{}", entry.id); test_support;
                            text_size: $crate::ui::SMALL; text_color: $palette.accent;
                        } [ $status_label ]
                    },
                    check_detail
                        .clone()
                        .unwrap_or_else(|| crate::i18n::tr!("agent-check-help").into()),
                )
            });
        let heading = gds_uim::el! {
            #div {
                id: $format!("agents-registry-heading-{}", entry.id); test_support;
                flex; items_center; gap_2; min_w_0;
            } [
                $Self::registry_icon(entry, &palette),
                #div { min_w_0; flex_1; flex; items_center; gap_2; } [
                    #div {
                        min_w_0; truncate; text_size: $crate::ui::SMALL;
                        font_weight: $FontWeight::SEMIBOLD;
                    } [ $entry.name.clone() ],
                    ..$bool::then(!entry.version.is_empty(), || gds_uim::el! {
                        #div { text_size: $crate::ui::SMALL; text_color: $palette.muted; } [
                            $entry.version.clone()
                        ]
                    }),
                    ..status_hint,
                ],
                $self.registry_actions(entry, cx),
            ]
        };
        gds_uim::el! {
            #div {
                id: $format!("agents-registry-{}", entry.id); test_support;
                w_full; min_w_0; flex; flex_col; gap_1; px_3; py_2;
                bg: $palette.card; border_1; border_color: $palette.border; rounded_md;
            } [
                $heading,
                #div {
                    id: $format!("agents-registry-details-{}", entry.id); test_support;
                    flex; items_center; gap_2; min_w_0; text_size: $crate::ui::SMALL;
                } [
                    #div { min_w_0; flex_1; truncate; text_color: $palette.muted; } [
                        $entry.description.clone()
                    ],
                    $self.registry_links(entry, &palette),
                ],
                ..$detail.map(|detail| gds_uim::el! {
                    #div {
                        id: $format!("agents-operation-{}", entry.id); test_support;
                        text_size: $crate::ui::SMALL; text_color: $palette.accent; min_w_0;
                    } [ $detail ]
                }),
                ..$entry.spec.as_ref().filter(|spec| {
                    matches!(
                        status,
                        Some(crate::agents::Status::AuthenticationRequired { .. })
                    ) || self.authentications.contains_key(&spec.id)
                }).map(|spec| {
                    let index = self
                        .catalog()
                        .iter()
                        .position(|configured| configured == spec)
                        .unwrap_or(0);
                    gds_uim::el! {
                        #div { pt_2; border_t_1; border_color: $palette.border; } [
                            $self.authentication_content(spec, index, cx)
                        ]
                    }
                }),
            ]
        }
        .into_any_element()
    }

    pub(super) fn registry_header(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = Palette::current(cx);
        let title = gds_uim::el! {
            #div { flex; items_center; justify_between; gap_2; } [
                #div { flex; items_center; gap_3; } [
                    #div { text_size: $crate::ui::TITLE; } [ $crate::i18n::tr!("catalog-title") ],
                    Button("agents-registry-learn-more") {
                        ghost; xsmall; label: $crate::i18n::tr!("action-learn-more");
                        text_color: $palette.accent; icon: $IconName::ExternalLink;
                        on_click: $cx.listener(Self::on_registry_learn_more);
                    },
                ],
                Button("agents-configure") {
                    ghost; xsmall; label: $crate::i18n::tr!("agent-advanced");
                    on_click: $cx.listener(Self::on_configure_agents);
                },
            ]
        };
        let filters = [
            ("agents-filter-all", crate::i18n::tr!("filter-all"), Filter::All),
            ("agents-filter-installed", crate::i18n::tr!("agent-installed"), Filter::Installed),
            ("agents-filter-not-installed", crate::i18n::tr!("agent-not-installed"), Filter::NotInstalled),
        ].map(|(id, label, filter)| {
            let selected = self.registry_filter == filter;
            gds_uim::el! {
                Button($id) {
                    label: $label; selected: $selected;
                    @selected => {
                        bg: $palette.action_fill; text_color: $palette.action_foreground;
                        border_color: $palette.action_fill;
                    };
                    on_click: $cx.listener(move |this, _, _, cx| this.set_registry_filter(filter, cx));
                }
            }
        }).map(Button::into_component);
        let toolbar = gds_uim::el! {
            #div { flex; items_center; justify_between; gap_3; } [
                ButtonGroup("agents-registry-filters") { outline; small; } [ ..filters ],
                Button("agents-registry-refresh") {
                    ghost; xsmall; icon: $IconName::RefreshCw;
                    label: $(if self.registry.loading() {
                        crate::i18n::tr!("catalog-refreshing")
                    } else {
                        crate::i18n::tr!("catalog-refresh")
                    });
                    disabled: $self.registry.loading();
                    on_click: $cx.listener(Self::on_refresh_registry);
                },
            ]
        };
        let search_icon = gds_uim::el! { Icon($IconName::Search) { small; } };
        gds_uim::el! {
            #div { flex; flex_col; gap_3; } [
                $title,
                #div {
                    id: "agents-registry-search"; test_support; w_full; p_1; bg: $palette.card;
                    border_1; border_color: $palette.border; rounded_md;
                } [
                    Input($&self.registry_search) {
                        appearance: false; cleanable: true; prefix: $search_icon;
                        aria_label: $crate::i18n::tr!("agent-search");
                    },
                ],
                $toolbar,
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

    fn on_configure_agents(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.show_installation = false;
        self.scroll_to = Some(0);
        cx.notify();
    }

    fn set_registry_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.registry_filter = filter;
        cx.notify();
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
        let entries: Vec<_> = self
            .registry
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .entries
                    .iter()
                    .filter(|entry| {
                        (entry.source == Source::Registry || !self.registry.editable())
                            && self.registry_filter.matches(entry)
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
        gds_uim::el! {
            #div { id: "agents-registry"; test_support; w_full; min_w_0; flex; flex_col; gap_2; } [
                ..$warning.map(|warning| gds_uim::el! {
                    #div { text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                        $warning.clone()
                    ]
                }),
                ..$entries.is_empty().then(|| gds_uim::el! {
                    #div { py_4; text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                        $(if self.registry.loading() {
                            crate::i18n::tr!("catalog-loading")
                        } else if self.registry.catalog.is_none() {
                            crate::i18n::tr!("catalog-connect")
                        } else {
                            crate::i18n::tr!("catalog-no-results")
                        })
                    ]
                }),
                ..$entries.into_iter().map(|entry| self.registry_card(entry, cx)),
            ]
        }.into_any_element()
    }
}

#[cfg(test)]
mod tests;
