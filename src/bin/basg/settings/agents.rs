//! Agent configuration and shared operations for the independent installation window.

use super::action_button as button;
use crate::{
    i18n::tr,
    settings,
    ui::{self, Button},
};
use gds_uim::el;
mod actions;
mod authentication;
mod feedback;
mod item;
mod logout;
mod registry;
mod removal;
mod status;
pub(super) mod window;

#[derive(Default)]
struct SharedAgents(HashMap<Option<bas::service::tls::ConnectionOptions>, Entity<Agents>>);
impl gpui_kit::Global for SharedAgents {}

pub(super) fn shared(window: &mut Window, cx: &mut App) -> Entity<Agents> {
    use gpui_kit::AppContext;
    let target = selected_target(cx);
    if let Some(agents) = cx
        .try_global::<SharedAgents>()
        .and_then(|shared| shared.0.get(&target))
    {
        return agents.clone();
    }
    let agents = cx.new(|cx| Agents::new(Store::new(None), window, cx));
    if !cx.has_global::<SharedAgents>() {
        cx.set_global(SharedAgents::default());
    }
    cx.update_global::<SharedAgents, _>(|shared, _| {
        shared.0.insert(target, agents.clone());
    });
    agents
}

fn selected_target(cx: &App) -> Option<bas::service::tls::ConnectionOptions> {
    cx.try_global::<catalog::Service>()
        .map(catalog::Service::address)
}
use crate::agents::{Status, catalog, check};
use bas::config::{
    AgentRegistration, DaemonDocument as Document, DaemonStore as Store,
    agent::{AgentSpec, Command, Kind},
    validate_registrations,
};
use gpui_kit::{
    App, Context, Entity, Render, ScrollHandle, Subscription, Task, TestSupportExt, Window,
    assets::IconName,
    component::{button::*, input::*, *},
    div,
    prelude::*,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

struct Check {
    spec: AgentSpec,
    status: Status,
    completed_at: Option<Instant>,
    _task: Option<Task<()>>,
}

#[derive(PartialEq, Eq)]
struct Form<T> {
    id: T,
    program: T,
    args: T,
    description: T,
    grace: T,
}

impl<T> Form<T> {
    fn fields(&self) -> [&T; 5] {
        [
            &self.id,
            &self.program,
            &self.args,
            &self.description,
            &self.grace,
        ]
    }
}

impl Form<Entity<InputState>> {
    fn new(window: &mut Window, cx: &mut Context<Agents>) -> Self {
        Self {
            id: cx.new(|cx| InputState::new(window, cx)),
            program: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(crate::i18n::tr!("agent-program-placeholder"))
            }),
            args: cx.new(|cx| InputState::new(window, cx).placeholder("[\"acp\"]")),
            description: cx.new(|cx| InputState::new(window, cx)),
            grace: cx.new(|cx| InputState::new(window, cx)),
        }
    }

    fn values(&self, cx: &App) -> Form<String> {
        Form {
            id: self.id.read(cx).value().to_string(),
            program: self.program.read(cx).value().to_string(),
            args: self.args.read(cx).value().to_string(),
            description: self.description.read(cx).value().to_string(),
            grace: self.grace.read(cx).value().to_string(),
        }
    }

    fn rows(
        &self,
        embedded: bool,
        connected: bool,
    ) -> Vec<(&'static str, &'static str, &Entity<InputState>)> {
        let mut rows = vec![("agents-id", "Agent ID", &self.id)];
        if !embedded {
            rows.push((
                "agents-program",
                crate::i18n::tr!("agent-program-label"),
                &self.program,
            ));
            rows.push((
                "agents-args",
                crate::i18n::tr!("agent-args-label"),
                &self.args,
            ));
            rows.push((
                "agents-description",
                crate::i18n::tr!("agent-description"),
                &self.description,
            ));
        }
        if connected {
            rows.push(("agents-grace", crate::i18n::tr!("agent-grace"), &self.grace));
        }
        rows
    }
}

fn values(spec: &AgentSpec) -> Form<String> {
    Form {
        id: spec.id.to_string(),
        program: spec
            .command
            .as_ref()
            .map(|command| command.program.to_string_lossy().into_owned())
            .unwrap_or_default(),
        args: serde_json::to_string(
            &spec
                .command
                .as_ref()
                .map(|command| command.args.as_slice())
                .unwrap_or_default(),
        )
        .expect("string arrays serialize"),
        description: spec.description.clone(),
        grace: spec.cancellation_grace.as_millis().to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
enum DraftError {
    #[error("{}", crate::i18n::tr!("settings-not-loaded"))]
    NotLoaded,
    #[error("{}", crate::i18n::tr!("agent-select-required"))]
    NotSelected,
    #[error("{}", crate::i18n::tr!("agent-invalid-args", error = .0.to_string()))]
    Arguments(#[source] serde_json::Error),
    #[error("{}", crate::i18n::tr!("agent-invalid-grace", error = .0.to_string()))]
    Grace(#[source] std::num::ParseIntError),
    #[error("{0}")]
    Validation(#[from] bas::config::RegistrationError),
    #[error("{0}")]
    Store(#[from] bas::config::Error),
    #[error("{}", crate::i18n::tr!("agent-local-required"))]
    ReadOnly,
    #[error("{}", crate::i18n::tr!("auth-config-changed"))]
    Changed,
}

pub(super) struct Agents {
    catalog_service: Option<catalog::Service>,
    check_service: Option<check::Service>,
    store: Store,
    document: Option<AgentDocument>,
    configuration: Option<bas::service::pb::AgentConfiguration>,
    configuration_task: Option<Task<()>>,
    editor: Editor,
    /// Keep native draft inputs and their subscriptions alive across page navigation.
    form: Form<Entity<InputState>>,
    message: String,
    saved: Option<AgentSpec>,
    registration_errors: String,
    /// Section the sidebar asked to reveal; consumed by the next render.
    pub(super) scroll_to: Option<usize>,
    scroll: ScrollHandle,
    installation_scroll: ScrollHandle,
    checks: HashMap<bas::model::AgentId, Check>,
    authentications: HashMap<bas::model::AgentId, Entity<crate::authentication::Authentication>>,
    authentication_owners: HashMap<bas::model::AgentId, authentication::Owner>,
    rendering_window: Option<gpui_kit::AnyWindowHandle>,
    registry: registry::Registry,
    pending_reveal: Option<String>,
    operations: HashMap<String, registry::Operation>,
    logouts: HashMap<bas::model::AgentId, logout::State>,
    registry_search: Entity<InputState>,
    pub(super) query: String,
    _subscriptions: Vec<Subscription>,
}

/// The baseline's ID identifies the custom registration even when cards reorder.
enum Editor {
    Idle,
    Add(AgentSpec),
    Edit {
        baseline: AgentSpec,
        intent: EditIntent,
    },
}

struct AgentDocument {
    settings: bas::config::Daemon,
    local: Option<Document>,
}

enum EditIntent {
    Update,
    Delete,
}

impl Editor {
    fn spec(&self) -> Option<&AgentSpec> {
        match self {
            Self::Idle => None,
            Self::Add(draft) => Some(draft),
            Self::Edit { baseline, .. } => Some(baseline),
        }
    }

    fn selected(&self) -> Option<&bas::model::AgentId> {
        match self {
            Self::Edit { baseline, .. } => Some(&baseline.id),
            Self::Idle | Self::Add(_) => None,
        }
    }

    fn deleting(&self) -> bool {
        matches!(
            self,
            Self::Edit {
                intent: EditIntent::Delete,
                ..
            }
        )
    }

    fn mark_delete(&mut self) {
        if let Self::Edit { intent, .. } = self {
            *intent = EditIntent::Delete;
        }
    }
}

/// One navigable section of the agents page.
pub(super) struct Section {
    pub id: String,
    pub title: String,
}

impl Agents {
    fn accept_configuration(
        &mut self,
        configuration: bas::service::pb::AgentConfiguration,
    ) -> Result<(), String> {
        let agents = serde_json::from_slice(&configuration.registrations_json)
            .map_err(|error| format!("Invalid agent configuration: {error}"))?;
        let settings = bas::config::Daemon {
            agents,
            ..Default::default()
        };
        self.document = Some(AgentDocument {
            settings,
            local: None,
        });
        self.configuration = Some(configuration);
        self.registration_errors.clear();
        Ok(())
    }

    fn fetch_configuration(&mut self, cx: &mut Context<Self>) {
        if self.configuration_task.is_some() {
            return;
        }
        if self.dirty(cx) {
            self.refresh_registry(false, cx);
            return;
        }
        let Some(service) = self.catalog_service.clone() else {
            return;
        };
        let request = service.configuration(None);
        self.configuration_task = Some(cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                page.configuration_task = None;
                match result.and_then(|configuration| page.accept_configuration(configuration)) {
                    Ok(()) => {
                        page.message.clear();
                        page.editor = Editor::Idle;
                        page.refresh_registry(false, cx);
                    }
                    Err(error) => {
                        page.message = error;
                        page.refresh_registry(false, cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn save_configuration(&mut self, cx: &mut Context<Self>) {
        if self.configuration_task.is_some() {
            return;
        }
        let Some(service) = self.catalog_service.clone() else {
            return;
        };
        let saved_id = self.form.id.read(cx).value().to_string();
        let deleting = self.editor.deleting();
        let request = self.candidate(cx).and_then(|agents| {
            let baseline = self.configuration.as_ref().ok_or(DraftError::NotLoaded)?;
            Ok(bas::service::pb::SaveAgentConfigurationRequest {
                profile: baseline.profile.clone(),
                revision: baseline.revision.clone(),
                registrations_json: serde_json::to_vec(&agents)
                    .expect("serializable registrations"),
            })
        });
        let request = match request {
            Ok(request) => service.configuration(Some(request)),
            Err(error) => {
                self.message = error.to_string();
                cx.notify();
                return;
            }
        };
        self.configuration_task = Some(cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                page.configuration_task = None;
                match result.and_then(|configuration| page.accept_configuration(configuration)) {
                    Ok(()) => {
                        page.editor = Editor::Idle;
                        page.message.clear();
                        page.notify_agent_change(None, cx);
                        page.saved = if deleting {
                            None
                        } else {
                            page.catalog()
                                .into_iter()
                                .find(|spec| spec.id.as_str() == saved_id)
                        };
                        if let Some(spec) = page.saved.clone() {
                            page.start_check(spec, true, cx);
                        }
                        page.refresh_registry(false, cx);
                    }
                    Err(error) => page.message = error,
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn target(&self) -> Option<bas::service::tls::ConnectionOptions> {
        self.catalog_service.as_ref().map(catalog::Service::address)
    }

    pub(super) fn settings_path(&self) -> Option<&std::path::Path> {
        self.catalog_service
            .as_ref()
            .filter(|service| service.local())
            .and_then(|_| self.registry.catalog.as_ref()?.settings_path.as_deref())
    }

    pub(super) fn localize(&self, window: &mut Window, cx: &mut Context<Self>) {
        for (input, placeholder) in [
            (&self.registry_search, crate::i18n::tr!("agent-search")),
            (
                &self.form.program,
                crate::i18n::tr!("agent-program-placeholder"),
            ),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        cx.notify();
    }

    pub(super) fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form = Form::new(window, cx);
        let registry_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(crate::i18n::tr!("agent-search")));
        let mut subscriptions: Vec<_> = form
            .fields()
            .into_iter()
            .map(|input| cx.subscribe(input, |_, _, _: &InputEvent, cx| cx.notify()))
            .collect();
        subscriptions.push(cx.subscribe(&registry_search, |_, _, _: &InputEvent, cx| cx.notify()));
        let mut page = Self {
            catalog_service: cx.try_global::<catalog::Service>().cloned(),
            check_service: cx.try_global::<check::Service>().cloned(),
            store,
            document: None,
            configuration: None,
            configuration_task: None,
            editor: Editor::Idle,
            form,
            message: String::new(),
            saved: None,
            registration_errors: String::new(),
            scroll_to: None,
            scroll: ScrollHandle::new(),
            installation_scroll: ScrollHandle::new(),
            checks: HashMap::new(),
            authentications: HashMap::new(),
            authentication_owners: HashMap::new(),
            rendering_window: None,
            registry: registry::Registry::new(cx.try_global::<catalog::Service>().is_some()),
            pending_reveal: None,
            operations: HashMap::new(),
            logouts: HashMap::new(),
            registry_search,
            query: String::new(),
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
    }

    fn catalog(&self) -> Vec<AgentSpec> {
        self.document
            .as_ref()
            .map(|document| {
                document
                    .settings
                    .agents
                    .iter()
                    .filter_map(AgentRegistration::as_custom)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn accept_document(&mut self, document: Document) {
        self.registration_errors = document
            .settings
            .agents
            .iter()
            .filter_map(|registration| {
                registration
                    .resolve()
                    .err()
                    .map(|error| format!("{}: {error}", registration.id()))
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.document = Some(AgentDocument {
            settings: document.settings.clone(),
            local: Some(document),
        });
    }

    pub(super) fn display_catalog(&self) -> Vec<AgentSpec> {
        self.configuration_entries()
            .into_iter()
            .filter_map(|entry| entry.spec)
            .collect()
    }

    pub(super) fn configuration_entries(&self) -> Vec<bas::management::Entry> {
        use bas::management::{Entry, Source, Status};
        let mut entries: Vec<_> = if self.configuration.is_some() {
            self.document
                .iter()
                .flat_map(|document| &document.settings.agents)
                .map(|registration| self.remote_configuration_entry(registration))
                .collect()
        } else if self.registry.editable() && self.catalog_service.is_none() {
            self.document
                .iter()
                .flat_map(|document| &document.settings.agents)
                .map(|registration| match registration.resolve() {
                    Ok(spec) => self.configured_entry(&spec),
                    Err(error) => {
                        let mut entry = self
                            .registry
                            .catalog
                            .as_ref()
                            .and_then(|catalog| {
                                catalog
                                    .entries
                                    .iter()
                                    .find(|entry| entry.id == registration.id().as_str())
                            })
                            .cloned()
                            .unwrap_or_else(|| Entry {
                                id: registration.id().to_string(),
                                name: registration.id().to_string(),
                                description: String::new(),
                                version: String::new(),
                                repository: String::new(),
                                license: String::new(),
                                license_url: String::new(),
                                icon: String::new(),
                                installed_version: None,
                                source: Source::Registry,
                                status: Status::Missing,
                                message: String::new(),
                                spec: None,
                            });
                        entry.source = Source::Registry;
                        entry.status = Status::Missing;
                        entry.spec = None;
                        entry.message = error.to_string();
                        entry
                    }
                })
                .collect()
        } else {
            self.registry
                .catalog
                .iter()
                .flat_map(|catalog| &catalog.entries)
                .filter(|entry| {
                    entry.source != Source::Registry
                        || entry.spec.is_some()
                        || entry.installed_version.is_some()
                        || matches!(entry.status, Status::Installed | Status::Missing)
                })
                .cloned()
                .collect()
        };
        entries.sort_by_key(|entry| match entry.source {
            bas::management::Source::Registry => 0,
            bas::management::Source::Custom => 1,
            bas::management::Source::Embedded => 2,
        });
        entries
    }

    fn remote_configuration_entry(
        &self,
        registration: &AgentRegistration,
    ) -> bas::management::Entry {
        if let Some(spec) = registration.as_custom() {
            return self.configured_entry(spec);
        }
        if let Some(entry) = self.registry.catalog.as_ref().and_then(|catalog| {
            catalog.entries.iter().find(|entry| {
                entry.id == registration.id().as_str()
                    && entry.source == bas::management::Source::Registry
            })
        }) {
            return entry.clone();
        }
        // Registration identity is available even when its server-side descriptor is not.
        bas::management::Entry {
            id: registration.id().to_string(),
            name: registration.id().to_string(),
            description: String::new(),
            version: String::new(),
            repository: String::new(),
            license: String::new(),
            license_url: String::new(),
            icon: String::new(),
            installed_version: None,
            source: bas::management::Source::Registry,
            status: bas::management::Status::Missing,
            message: crate::i18n::tr!("agent-catalog-missing").into(),
            spec: None,
        }
    }

    pub(super) fn open_installation(&mut self, cx: &mut Context<Self>) {
        self.pending_reveal = None;
        self.refresh_registry(false, cx);
        let agents = cx.entity();
        cx.defer(move |cx| {
            if let Err(error) = window::open(agents.clone(), cx) {
                agents.update(cx, |agents, cx| {
                    agents.message = error.to_string();
                    cx.notify();
                });
            }
        });
        cx.notify();
    }

    pub(super) fn show_section(&mut self, section: Option<usize>, cx: &mut Context<Self>) {
        self.pending_reveal = None;
        self.scroll_to = Some(section.unwrap_or(0));
        self.query.clear();
        cx.notify();
    }

    pub(super) fn reveal(&mut self, id: &str, cx: &mut Context<Self>) -> Option<usize> {
        self.pending_reveal = None;
        let managed = self.document.as_ref().and_then(|document| {
            document
                .settings
                .agents
                .iter()
                .find(|registration| registration.id().as_str() == id)
                .and_then(AgentRegistration::managed)
                .cloned()
        });
        if let Some(managed) = managed {
            let index = self
                .configuration_entries()
                .iter()
                .position(|entry| entry.id == id)
                .unwrap_or(0);
            self.scroll_to = Some(index);
            if self.registry.catalog.is_none() && !self.registry.editable() {
                // Recognize ownership immediately, but wait for the daemon's profile check
                // before launching a connection/authentication probe.
                self.pending_reveal = Some(id.to_owned());
                self.refresh_registry(false, cx);
            } else if self.catalog_service.is_some() {
                if let Some(spec) = self
                    .registry
                    .catalog
                    .as_ref()
                    .and_then(|catalog| catalog.entries.iter().find(|entry| entry.id == id))
                    .and_then(|entry| entry.spec.clone())
                {
                    self.start_check(spec, true, cx);
                }
            } else {
                match managed.resolve() {
                    Ok(spec) => self.start_check(spec, true, cx),
                    Err(error) => self.message = error.to_string(),
                }
            }
            cx.notify();
            return Some(index);
        }
        if let Some(entry) = self.registry.catalog.as_ref().and_then(|catalog| {
            catalog
                .entries
                .iter()
                .find(|entry| entry.id == id && entry.source == bas::management::Source::Registry)
        }) {
            let spec = entry.spec.clone();
            let index = self
                .configuration_entries()
                .iter()
                .position(|entry| entry.id == id)
                .unwrap_or(0);
            self.scroll_to = Some(index);
            if let Some(spec) = spec {
                self.start_check(spec, true, cx);
            }
            cx.notify();
            return Some(index);
        }
        let entry = self
            .configuration_entries()
            .into_iter()
            .enumerate()
            .find(|(_, entry)| entry.id == id);
        if let Some((index, entry)) = entry {
            self.scroll_to = Some(index);
            if let Some(spec) = entry.spec {
                self.start_check(spec, true, cx);
            }
            Some(index)
        } else if self.registry.awaiting_profile() {
            self.pending_reveal = Some(id.to_owned());
            self.refresh_registry(false, cx);
            cx.notify();
            None
        } else {
            self.message = crate::i18n::tr!("agent-config-absent").into();
            cx.notify();
            None
        }
    }

    fn start_check(&mut self, spec: AgentSpec, connect: bool, cx: &mut Context<Self>) {
        if self.logging_out(&spec) {
            return;
        }
        self.logouts.remove(&spec.id);
        if spec.kind == Kind::Embedded {
            self.checks.insert(
                spec.id.clone(),
                Check {
                    spec,
                    status: Status::Connected(String::new()),
                    completed_at: None,
                    _task: None,
                },
            );
            return;
        }
        let Some(service) = self.check_service.clone() else {
            self.checks.insert(
                spec.id.clone(),
                Check {
                    spec,
                    status: Status::Failed(crate::i18n::tr!("agent-check-unavailable").into()),
                    completed_at: Some(Instant::now()),
                    _task: None,
                },
            );
            return;
        };
        self.authentications.remove(&spec.id);
        let request = service.request(spec.clone(), connect, self.store.path().to_owned());
        let expected = spec.clone();
        let task = cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                page.apply_check(&expected, result);
                if page.status(&expected).is_some_and(Status::connected) {
                    page.notify_agent_change(Some(expected.id.to_string()), cx);
                } else if matches!(
                    page.status(&expected),
                    Some(Status::AuthenticationRequired { .. })
                ) {
                    page.scroll_to = page
                        .display_catalog()
                        .iter()
                        .position(|spec| spec.id == expected.id);
                }
                cx.notify();
            });
        });
        self.checks.insert(
            spec.id.clone(),
            Check {
                spec,
                status: if connect {
                    Status::Connecting
                } else {
                    Status::Checking
                },
                completed_at: None,
                _task: Some(task),
            },
        );
        cx.notify();
    }

    fn apply_check(&mut self, expected: &AgentSpec, result: Status) {
        if let Some(check) = self.checks.get_mut(&expected.id)
            && check.spec == *expected
        {
            check.completed_at =
                (!result.busy() && !matches!(result, Status::Installed(_))).then(Instant::now);
            check.status = result;
        }
    }

    fn status(&self, spec: &AgentSpec) -> Option<&Status> {
        self.checks
            .get(&spec.id)
            .filter(|check| check.spec == *spec)
            .map(|check| &check.status)
    }

    fn check_detail(&self, spec: &AgentSpec) -> String {
        if self.authentications.contains_key(&spec.id) {
            return crate::i18n::tr!("agent-auth-help").into();
        }
        if spec.kind == Kind::Embedded {
            return crate::i18n::tr!("agent-builtin-help").into();
        }
        let check = self
            .checks
            .get(&spec.id)
            .filter(|check| check.spec == *spec);
        let age = check
            .and_then(|check| check.completed_at)
            .map(|at| crate::ui::time::relative_age(at.elapsed()));
        let previous = crate::i18n::tr!(
            "agent-last-check",
            time = (age
                .as_deref()
                .unwrap_or(crate::i18n::tr!("time-settings-open")))
            .to_string()
        );
        match check.map(|check| &check.status) {
            Some(Status::Installed(_)) => crate::i18n::tr!("agent-check-help").into(),
            Some(Status::Connected(_)) => {
                crate::i18n::tr!("agent-check-ready", previous = (previous).to_string())
            }
            Some(Status::AuthenticationRequired { .. }) => {
                crate::i18n::tr!("agent-check-signin", previous = (previous).to_string())
            }
            Some(Status::Missing(message) | Status::Failed(message)) => {
                format!("{previous}：{message}")
            }
            Some(Status::Authenticating) => crate::i18n::tr!("agent-auth-recheck").into(),
            Some(Status::Checking) => crate::i18n::tr!("agent-locating").into(),
            Some(Status::Connecting) => crate::i18n::tr!("agent-testing").into(),
            None => crate::i18n::tr!("agent-check-help").into(),
        }
    }

    fn draft_connected(&self, cx: &App) -> bool {
        let Some(draft) = self.editor.spec() else {
            return false;
        };
        if draft.kind == Kind::Embedded {
            return true;
        }
        self.checks.get(&draft.id).is_some_and(|check| {
            let checked = values(&check.spec);
            let form = self.form.values(cx);
            check.status.connected()
                && checked.id == form.id
                && checked.program == form.program
                && checked.args == form.args
                && check.spec.env == draft.env
        })
    }

    /// Sidebar entries contain configuration only.
    pub(super) fn sections(&self) -> Vec<Section> {
        self.configuration_entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| Section {
                id: format!("settings-agent-{index}"),
                title: entry.id.clone(),
            })
            .collect()
    }

    fn dirty(&self, cx: &App) -> bool {
        self.editor.spec().is_some_and(|draft| {
            matches!(self.editor, Editor::Add(_))
                || self.editor.deleting()
                || self.form.values(cx) != values(draft)
        })
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.catalog_service.is_some() {
            if self.dirty(cx) {
                self.message = crate::i18n::tr!("agent-save-before-reload").into();
            } else {
                self.fetch_configuration(cx);
            }
            cx.notify();
            return;
        }
        if self.dirty(cx) {
            self.message = crate::i18n::tr!("agent-save-before-reload").into();
        } else {
            match self.store.load() {
                Ok(document) => {
                    self.accept_document(document);
                    self.editor = Editor::Idle;
                    self.message.clear();
                    self.checks.clear();
                    if self.catalog_service.is_some() {
                        self.refresh_registry(false, cx);
                    } else {
                        for spec in self.catalog() {
                            self.start_check(spec, false, cx);
                        }
                    }
                }
                Err(error) => {
                    self.message =
                        crate::i18n::tr!("settings-load-error", error = (error).to_string())
                }
            }
        }
        cx.notify();
    }

    fn edit(
        &mut self,
        spec: AgentSpec,
        selected: Option<bas::model::AgentId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.registry.editable() || self.configuration_task.is_some() {
            self.message = crate::i18n::tr!("agent-edit-mismatch").into();
        } else if selected
            .as_ref()
            .is_some_and(|id| id != &spec.id || !self.catalog().iter().any(|saved| saved == &spec))
            || self.document.as_ref().is_some_and(|document| {
                document.settings.agents.iter().any(|registration| {
                    registration.id() == &spec.id && registration.managed().is_some()
                })
            })
        {
            self.message = DraftError::Changed.to_string();
        } else if self.dirty(cx) {
            self.message = crate::i18n::tr!("agent-save-before-switch").into();
        } else {
            self.saved = None;
            let form = values(&spec);
            for (input, value) in self.form.fields().into_iter().zip(form.fields()) {
                input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
            self.scroll_to = Some(
                selected
                    .as_ref()
                    .and_then(|id| {
                        self.configuration_entries()
                            .iter()
                            .position(|entry| entry.id == id.as_str())
                    })
                    .unwrap_or_else(|| self.configuration_entries().len()),
            );
            self.editor = if selected.is_some() {
                Editor::Edit {
                    baseline: spec,
                    intent: EditIntent::Update,
                }
            } else {
                Editor::Add(spec)
            };
            self.message.clear();
        }
        cx.notify();
    }

    /// Build the complete catalog without modifying the document or disk.
    ///
    /// # Errors
    /// Rejects missing editor state, malformed form values, and invalid catalogs.
    fn candidate(&self, cx: &App) -> Result<Vec<AgentRegistration>, DraftError> {
        if !self.registry.editable() {
            return Err(DraftError::ReadOnly);
        }
        let mut catalog = self
            .document
            .as_ref()
            .ok_or(DraftError::NotLoaded)?
            .settings
            .agents
            .clone();
        let mut spec = self.editor.spec().cloned().ok_or(DraftError::NotSelected)?;
        let selected = self
            .editor
            .selected()
            .map(|id| {
                catalog
                    .iter()
                    .enumerate()
                    .find(|(_, registration)| registration.id() == id)
                    .filter(|(_, registration)| registration.as_custom() == Some(&spec))
                    .map(|(index, _)| index)
                    .ok_or(DraftError::Changed)
            })
            .transpose()?;
        if self.editor.deleting() {
            catalog.remove(selected.ok_or(DraftError::NotSelected)?);
        } else {
            let form = self.form.values(cx);
            spec.id = form.id.into();
            spec.description = form.description.trim().to_owned();
            if spec.kind == Kind::Acp {
                let program = form.program;
                let args = serde_json::from_str::<Vec<String>>(&form.args)
                    .map_err(DraftError::Arguments)?;
                // Preserve non-UTF8 paths when the user has not edited the displayed program.
                if values(&spec).program != program {
                    spec.command = Some(Command::new(program, args));
                } else if let Some(command) = spec.command.as_mut() {
                    command.args = args;
                }
            }
            if self.draft_connected(cx) {
                let millis = form.grace.parse::<u64>().map_err(DraftError::Grace)?;
                spec.cancellation_grace = Duration::from_millis(millis);
            }
            if let Some(index) = selected {
                catalog[index] = spec.into();
            } else {
                catalog.push(spec.into());
            }
        }
        validate_registrations(&catalog)?;
        Ok(catalog)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.configuration_task.is_some() {
            return;
        }
        if self.catalog_service.is_some() {
            self.save_configuration(cx);
            return;
        }
        let connected = self.draft_connected(cx);
        let deleting = self.editor.deleting();
        let saved_id: bas::model::AgentId = self.form.id.read(cx).value().to_string().into();
        let result = self.candidate(cx).and_then(|catalog| {
            self.store
                .save_agents(
                    self.document
                        .as_mut()
                        .and_then(|document| document.local.as_mut())
                        .ok_or(DraftError::NotLoaded)?,
                    catalog,
                )
                .map_err(DraftError::Store)
        });
        match result {
            Ok(()) => {
                if let Some(document) = self.document.as_mut()
                    && let Some(local) = &document.local
                {
                    document.settings = local.settings.clone();
                }
                self.editor = Editor::Idle;
                self.saved = None;
                self.message.clear();
                self.notify_agent_change(None, cx);
                let catalog = self.catalog();
                if let Some(document) = &self.document {
                    self.checks.retain(|id, _| {
                        document
                            .settings
                            .agents
                            .iter()
                            .any(|registration| registration.id() == id)
                    });
                }
                if !deleting
                    && let Some(spec) = catalog.into_iter().find(|spec| spec.id == saved_id)
                {
                    self.saved = Some(spec.clone());
                    if connected {
                        // Changing an advanced value does not change the checked transport.
                        if let Some(check) = self.checks.get_mut(&spec.id) {
                            check.spec = spec;
                        }
                    } else {
                        self.start_check(spec, true, cx);
                    }
                }
            }
            Err(error) => {
                self.message = crate::i18n::tr!("agent-save-error", error = (error).to_string());
            }
        }
        cx.notify();
    }
}

/// Display metadata, without substituting launch details for an empty note.
fn summary(spec: &AgentSpec) -> String {
    match spec.kind {
        Kind::Embedded if spec.description.is_empty() => {
            crate::i18n::tr!("agent-embedded-description").to_owned()
        }
        _ => spec.description.clone(),
    }
}

impl Agents {
    fn source(&self, spec: &AgentSpec) -> bas::management::Source {
        use bas::management::Source;
        if spec.kind == Kind::Embedded {
            return Source::Embedded;
        }
        if self.registry.editable()
            && let Some(registration) = self
                .document
                .as_ref()
                .and_then(|document| document.settings.agents.iter().find(|r| r.id() == &spec.id))
        {
            return if registration.managed().is_some() {
                Source::Registry
            } else {
                Source::Custom
            };
        }
        self.registry
            .catalog
            .as_ref()
            .and_then(|catalog| {
                catalog
                    .entries
                    .iter()
                    .find(|entry| entry.id == spec.id.as_str())
            })
            .map_or(Source::Custom, |entry| entry.source)
    }

    fn configured_entry(&self, spec: &AgentSpec) -> bas::management::Entry {
        if let Some(entry) = self.registry.catalog.as_ref().and_then(|catalog| {
            catalog
                .entries
                .iter()
                .find(|entry| entry.id == spec.id.as_str())
        }) {
            let mut entry = entry.clone();
            entry.spec = Some(spec.clone());
            entry.source = self.source(spec);
            if entry.source == bas::management::Source::Custom {
                entry.description = summary(spec);
            }
            return entry;
        }
        bas::management::Entry {
            id: spec.id.to_string(),
            name: spec.id.to_string(),
            description: summary(spec),
            version: String::new(),
            repository: String::new(),
            license: String::new(),
            license_url: String::new(),
            icon: String::new(),
            installed_version: None,
            source: self.source(spec),
            status: bas::management::Status::Installed,
            message: String::new(),
            spec: Some(spec.clone()),
        }
    }

    fn configuration_actions(
        &self,
        index: usize,
        entry: &bas::management::Entry,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if entry.source == bas::management::Source::Registry || entry.spec.is_none() {
            return self.registry_actions(entry, cx);
        }
        let spec = entry
            .spec
            .as_ref()
            .expect("configured custom or embedded agent");
        let edit = spec.clone();
        let custom = entry.source == bas::management::Source::Custom;
        el! {
            .div { flex; items_center; gap_1; } [
                @if (spec.kind == Kind::Acp)
                    $self.check_button(format!("agents-check-{index}"), spec, cx),
                @end,
                $self.account_actions(spec, cx),
                @if (custom)
                    (format!("agents-edit-{index}"), IconName::Settings, tr!("agent-edit-config")).item::action {
                        disabled: $(!self.registry.editable() || !custom);
                        on_click: $cx.listener(move |this, _, window, cx| {
                            this.edit(edit.clone(), Some(edit.id.clone()), window, cx)
                        });
                    },
                    $self.remove_button(spec.id.as_str(), cx),
                @end,
            ]
        }.into_any_element()
    }

    fn configuration_card(
        &self,
        index: usize,
        entry: bas::management::Entry,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let spec = entry.spec.as_ref();
        let editing = self.registry.editable()
            && spec.is_some_and(|spec| self.editor.selected() == Some(&spec.id));
        let content = if editing {
            Some(self.form_content(cx))
        } else {
            spec.filter(|spec| self.authentications.contains_key(&spec.id))
                .map(|spec| self.authentication_content(spec, index, cx))
        };
        let card = item::Item {
            status: None,
            status_hint: String::new(),
            feedback: self.item_feedback(&entry),
            actions: self.configuration_actions(index, &entry, cx),
            entry,
            content,
        };
        el! {
            .div {
                # format!("agents-card-{index}"); test_support;
                w_full; min_w_0;
            } [card]
        }
        .into_any_element()
    }

    fn cancel_button(&self, cx: &mut Context<Self>) -> crate::ui::Button {
        button("agents-cancel")
            .label(crate::i18n::tr!("action-cancel"))
            .disabled(self.configuration_task.is_some())
            .on_click(cx.listener(Self::on_cancel_edit))
    }

    fn form_content(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let embedded = self
            .editor
            .spec()
            .is_some_and(|spec| spec.kind == Kind::Embedded);
        let connected = self.draft_connected(cx);
        let actions = el! {
            .div { # "agents-form-actions"; test_support; w_full; flex; items_center; gap_2; } [
                .div { flex_1; },
                $self.cancel_button(cx),
                ("agents-save").button {
                    disabled: $(!self.registry.editable() || self.document.is_none() || self.configuration_task.is_some());
                    label: tr!("agent-save");
                    on_click: (cx, Self::on_save).Context::listener;
                },
            ]
        };
        el! {
            .div { flex; flex_col; gap_4; } [
                @if (!embedded && !connected)
                    .div {
                        # "agents-connection-help"; test_support;
                        text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
                    } [tr!("agent-custom-help")],
                @end,
                ..$(self.form.rows(embedded, connected)) { into_iter; map: ((id, label, input),) =>
                    .div { flex; flex_col; gap_1; } [
                        .div { text_size: ui::SMALL; } [label],
                        .div { # id; test_support; } [
                            $ui::input::placeholder(input, Input::new(input).small().disabled(self.editor.deleting() || !self.registry.editable() || self.configuration_task.is_some()))
                        ],
                    ]
                },
                @if (self.editor.deleting())
                    .div { text_size: ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                        tr!("agent-remove-confirm-help")
                    ],
                @end,
                @if (!self.message.is_empty())
                    ("agents-form-message", ~$(self.message), $cx.theme().danger).settings::feedback,
                @end,
                actions,
            ]
        }.into_any_element()
    }

    fn on_save(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn on_cancel_edit(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.configuration_task.is_some() {
            return;
        }
        self.editor = Editor::Idle;
        if let Some(catalog) = self.registry.catalog.clone()
            && self.catalog_service.is_some()
        {
            self.apply_catalog(catalog, cx);
        }
        self.message = crate::i18n::tr!("agent-edit-cancelled").into();
        cx.notify();
    }

    fn on_add_custom(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(AgentSpec::acp("", Command::new("", [])), None, window, cx);
    }

    fn on_add_agent(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.open_installation(cx);
    }

    fn on_reload_agents(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reload(cx);
    }
}

impl Render for Agents {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_page(Page::Configuration, window, cx)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Configuration,
    Onboarding,
}

impl Agents {
    fn render_page(
        &mut self,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        self.rendering_window = Some(window.window_handle());
        self.ensure_authentication_panels(page, window, cx);
        let dirty = self.dirty(cx);
        let catalog = self.configuration_entries();
        let drafting = matches!(self.editor, Editor::Add(_));
        let section_count = if page == Page::Onboarding {
            2
        } else {
            catalog.len() + 1
        };
        if page == Page::Configuration
            && let Some(index) = self.scroll_to.take()
        {
            self.scroll
                .scroll_to_top_of_item(index.min(section_count.saturating_sub(1)));
        }
        let header = if page == Page::Onboarding && drafting {
            el! {
                .div { flex; items_center; gap_2; } [
                    .div { text_size: ui::HEADING; } [
                        tr!("agent-add-custom")
                    ],
                ]
            }
            .into_any_element()
        } else if page == Page::Onboarding {
            self.registry_header(cx)
        } else {
            el! {
                .div {
                    # "agents-configuration-header"; test_support;
                    flex; items_center; justify_between; gap_3;
                } [
                    .div { text_size: ui::HEADING; } [
                        tr!("settings-agents")
                    ],
                    .div {
                        # "agents-configuration-actions"; test_support;
                        flex; items_center; gap_2;
                    } [
                        ("agents-add").Button::new {
                            ghost; small; icon: IconName::Plus;
                            label: tr!("agent-add");
                            on_click: (cx, Self::on_add_agent).Context::listener;
                        },
                        ("agents-reload", IconName::RefreshCw, tr!("settings-refresh-agents")).item::action {
                            disabled: $self.configuration_task.is_some();
                            loading: $self.configuration_task.is_some();
                            on_click: (cx, Self::on_reload_agents).Context::listener;
                        },
                    ],
                ]
            }
            .into_any_element()
        };
        let mut content = Vec::new();
        if page == Page::Onboarding {
            if drafting {
                content.push(self.form_content(cx));
            } else {
                if let Some((index, spec)) = catalog
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| entry.spec.as_ref().map(|spec| (index, spec)))
                    .find(|(_, spec)| {
                        self.saved.as_ref() == Some(spec)
                            && self.authentications.contains_key(&spec.id)
                    })
                {
                    content.push(self.authentication_content(spec, index, cx));
                }
                content.push(self.registry_content(cx));
            }
        } else {
            let mut previous = None;
            for (index, entry) in catalog.into_iter().enumerate() {
                let source = entry.source;
                let heading = (previous != Some(source)).then(|| {
                    let (id, label) = match source {
                        bas::management::Source::Registry => ("agents-source-registry", crate::i18n::tr!("agent-source-registry")),
                        bas::management::Source::Custom => ("agents-source-custom", crate::i18n::tr!("agent-source-custom")),
                        bas::management::Source::Embedded => ("agents-source-embedded", crate::i18n::tr!("agent-builtin")),
                    };
                    el! { .div { # id; test_support; role: gpui_kit::Role::Label; aria_label: label; py_2; text_size: ui::HEADING; } [label] }
                });
                previous = Some(source);
                content.push(el! { .div { flex; flex_col; gap_2; } [..heading, $self.configuration_card(index, entry, cx)] }.into_any_element());
            }
        }
        let heading = el! {
            .div { p_5; flex_shrink_0; flex; flex_col; gap_2; } [
                header,
                @if (dirty)
                    ("agents-dirty", tr!("settings-unsaved"), $cx.theme().foreground).settings::feedback,
                @end,
                @if (!self.message.is_empty() && self.editor.spec().is_none())
                    ("agents-message", ~$(self.message), $cx.theme().muted_foreground).settings::feedback,
                @end,
                @if (page == Page::Onboarding)
                    ..$self.saved_feedback(cx),
                @end,
                @if (!self.registration_errors.is_empty())
                    ("agents-registration-errors", ~$(self.registration_errors), $cx.theme().foreground).settings::feedback,
                @end,
            ]
        };
        el! {
            .div {
                # "agents-page"; test_support; size_full; flex; flex_col;
                bg: $registry::Palette::current(cx).background;
            } [
                heading,
                .div {
                    # "agents-sections"; test_support; flex_1; min_h_0; overflow_y_scroll;
                    track_scroll: @if (page == Page::Configuration) $&self.scroll @else $&self.installation_scroll @end;
                    px_5; pb_5; flex; flex_col; gap_2;
                } [ ..content ],
            ]
        }.into_any_element()
    }
}
