//! Local catalog onboarding; only an explicit detection action starts an ACP probe.

use super::action_button as button;
use crate::ui::Button;
mod authentication;
mod registry;
mod status;
use crate::agents::{Status, catalog, check};
use bas::config::{
    AgentRegistration, Document, Store,
    agent::{AgentSpec, Command, Kind},
    validate_registrations,
};
use gpui_kit::{
    App, Context, Entity, Render, ScrollHandle, Subscription, Task, TestSupportExt, Window,
    component::{button::*, group_box::*, input::*, *},
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
    grace: T,
}

impl<T> Form<T> {
    fn fields(&self) -> [&T; 4] {
        [&self.id, &self.program, &self.args, &self.grace]
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
            grace: cx.new(|cx| InputState::new(window, cx)),
        }
    }

    fn values(&self, cx: &App) -> Form<String> {
        Form {
            id: self.id.read(cx).value().to_string(),
            program: self.program.read(cx).value().to_string(),
            args: self.args.read(cx).value().to_string(),
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
    store: Store,
    document: Option<Document>,
    editor: Editor,
    /// Keep native draft inputs and their subscriptions alive across page navigation.
    form: Form<Entity<InputState>>,
    message: String,
    registration_errors: String,
    /// Section the sidebar asked to reveal; consumed by the next render.
    pub(super) scroll_to: Option<usize>,
    scroll: ScrollHandle,
    checks: HashMap<bas::model::AgentId, Check>,
    authentications: HashMap<bas::model::AgentId, Entity<crate::authentication::Authentication>>,
    show_installation: bool,
    registry: registry::Registry,
    pending_reveal: Option<String>,
    operations: HashMap<String, registry::Operation>,
    registry_search: Entity<InputState>,
    registry_filter: registry::Filter,
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
            store,
            document: None,
            editor: Editor::Idle,
            form,
            message: String::new(),
            registration_errors: String::new(),
            scroll_to: None,
            scroll: ScrollHandle::new(),
            checks: HashMap::new(),
            authentications: HashMap::new(),
            show_installation: true,
            registry: registry::Registry::new(cx.try_global::<catalog::Service>().is_some()),
            pending_reveal: None,
            operations: HashMap::new(),
            registry_search,
            registry_filter: registry::Filter::All,
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
                    .daemon
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
            .daemon
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
        self.document = Some(document);
    }

    fn display_catalog(&self) -> Vec<AgentSpec> {
        if !self.registry.editable()
            && let Some(catalog) = &self.registry.catalog
        {
            return catalog
                .entries
                .iter()
                .filter(|entry| entry.source != bas::management::Source::Registry)
                .filter_map(|entry| entry.spec.clone())
                .collect();
        }
        self.catalog()
    }

    pub(super) fn open_installation(&mut self, cx: &mut Context<Self>) {
        self.pending_reveal = None;
        self.show_installation = true;
        self.scroll_to = Some(0);
        self.refresh_registry(false, cx);
        cx.notify();
    }

    pub(super) fn show_section(&mut self, section: Option<usize>, cx: &mut Context<Self>) {
        self.pending_reveal = None;
        self.show_installation = section.is_none_or(|index| {
            index >= self.display_catalog().len() && self.editor.spec().is_none()
        });
        self.scroll_to = Some(if self.show_installation {
            0
        } else {
            section.unwrap_or(0)
        });
        self.query.clear();
        cx.notify();
    }

    pub(super) fn reveal(&mut self, id: &str, cx: &mut Context<Self>) -> Option<usize> {
        self.pending_reveal = None;
        let managed = self.document.as_ref().and_then(|document| {
            document
                .settings
                .daemon
                .agents
                .iter()
                .find(|registration| registration.id().as_str() == id)
                .and_then(AgentRegistration::managed)
                .cloned()
        });
        if let Some(managed) = managed {
            self.show_installation = true;
            self.scroll_to = Some(0);
            if self.registry.catalog.is_none() && !self.registry.editable() {
                // Recognize ownership immediately, but wait for the daemon's profile check
                // before launching a connection/authentication probe.
                self.pending_reveal = Some(id.to_owned());
                self.refresh_registry(false, cx);
            } else {
                match managed.resolve() {
                    Ok(spec) => self.start_check(spec, true, cx),
                    Err(error) => self.message = error.to_string(),
                }
            }
            cx.notify();
            return Some(0);
        }
        if let Some(entry) = self.registry.catalog.as_ref().and_then(|catalog| {
            catalog
                .entries
                .iter()
                .find(|entry| entry.id == id && entry.source == bas::management::Source::Registry)
        }) {
            let spec = entry.spec.clone();
            self.show_installation = true;
            self.scroll_to = Some(0);
            if let Some(spec) = spec {
                self.start_check(spec, true, cx);
            }
            cx.notify();
            return Some(0);
        }
        let entry = self
            .catalog()
            .into_iter()
            .enumerate()
            .find(|(_, spec)| spec.id.as_str() == id);
        if let Some((index, spec)) = entry {
            self.show_installation = false;
            self.scroll_to = Some(index);
            self.start_check(spec, true, cx);
            Some(index)
        } else {
            self.message = crate::i18n::tr!("agent-config-absent").into();
            cx.notify();
            None
        }
    }

    fn start_check(&mut self, spec: AgentSpec, connect: bool, cx: &mut Context<Self>) {
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
        let Some(service) = cx.try_global::<check::Service>().cloned() else {
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
        if !self.registry.editable() {
            self.message = crate::i18n::tr!("agent-catalog-readonly").into();
            cx.notify();
            return;
        }
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
                ) && !page.show_installation
                {
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
        let age = check.and_then(|check| check.completed_at).map(|at| {
            let seconds = at.elapsed().as_secs();
            if seconds < 60 {
                crate::i18n::tr!("time-just-now").into()
            } else {
                crate::i18n::tr!("time-minutes-ago", count = seconds / 60)
            }
        });
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

    /// Sidebar entries: every configured agent, then the new ACP form.
    pub(super) fn sections(&self) -> Vec<Section> {
        let mut sections: Vec<_> = self
            .display_catalog()
            .iter()
            .enumerate()
            .map(|(index, spec)| Section {
                id: format!("settings-agent-{index}"),
                title: spec.id.to_string(),
            })
            .collect();
        sections.push(Section {
            id: "settings-agents-add".to_owned(),
            title: crate::i18n::tr!("agent-add").to_owned(),
        });
        sections
    }

    fn dirty(&self, cx: &App) -> bool {
        self.editor.spec().is_some_and(|draft| {
            matches!(self.editor, Editor::Add(_))
                || self.editor.deleting()
                || self.form.values(cx) != values(draft)
        })
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.dirty(cx) {
            self.message = crate::i18n::tr!("agent-save-before-reload").into();
        } else {
            match self.store.load() {
                Ok(document) => {
                    self.accept_document(document);
                    self.editor = Editor::Idle;
                    self.message.clear();
                    self.checks.clear();
                    if cx.try_global::<catalog::Service>().is_some() {
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
        if !self.registry.editable() {
            self.message = crate::i18n::tr!("agent-edit-mismatch").into();
        } else if selected
            .as_ref()
            .is_some_and(|id| id != &spec.id || !self.catalog().iter().any(|saved| saved == &spec))
            || self.document.as_ref().is_some_and(|document| {
                document.settings.daemon.agents.iter().any(|registration| {
                    registration.id() == &spec.id && registration.managed().is_some()
                })
            })
        {
            self.message = DraftError::Changed.to_string();
        } else if self.dirty(cx) {
            self.message = crate::i18n::tr!("agent-save-before-switch").into();
        } else {
            self.show_installation = false;
            let form = values(&spec);
            for (input, value) in self.form.fields().into_iter().zip(form.fields()) {
                input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
            self.scroll_to = Some(
                selected
                    .as_ref()
                    .and_then(|id| self.catalog().iter().position(|spec| &spec.id == id))
                    .unwrap_or_else(|| self.catalog().len()),
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
            .daemon
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
        let connected = self.draft_connected(cx);
        let deleting = self.editor.deleting();
        let saved_id: bas::model::AgentId = self.form.id.read(cx).value().to_string().into();
        let result = self.candidate(cx).and_then(|catalog| {
            self.store
                .save_agents(
                    self.document.as_mut().ok_or(DraftError::NotLoaded)?,
                    catalog,
                )
                .map_err(DraftError::Store)
        });
        match result {
            Ok(()) => {
                self.editor = Editor::Idle;
                self.message = crate::i18n::tr!("settings-saved").into();
                self.notify_agent_change(None, cx);
                let catalog = self.catalog();
                if let Some(document) = &self.document {
                    self.checks.retain(|id, _| {
                        document
                            .settings
                            .daemon
                            .agents
                            .iter()
                            .any(|registration| registration.id() == id)
                    });
                }
                if !deleting
                    && let Some(spec) = catalog.into_iter().find(|spec| spec.id == saved_id)
                {
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

/// One-line description of how a backend is driven.
fn summary(spec: &AgentSpec) -> String {
    match (spec.kind, spec.command()) {
        (Kind::Embedded, _) => crate::i18n::tr!("agent-embedded-description").to_owned(),
        (Kind::Acp, Some(command)) => format!("acp · {command}"),
        (Kind::Acp, None) => crate::i18n::tr!("agent-command-missing").to_owned(),
    }
}

impl Agents {
    /// A section card: title, one-line description, then the section body.
    fn card(
        &self,
        id: String,
        title: String,
        description: String,
        content: gpui_kit::AnyElement,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let heading = gds_uim::el! {
            #div { flex; flex_col; gap_1; } [
                #div {
                    text_size: $crate::ui::HEADING; text_color: $cx.theme().foreground;
                } [ $title ],
                #div {
                    text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                } [ $description ],
            ]
        };
        gds_uim::el! {
            #div { id: $id; test_support; } [
                GroupBox { outline; title: $heading; } [ $content ],
            ]
        }
        .into_any_element()
    }

    fn summary_content(
        &self,
        index: usize,
        spec: &AgentSpec,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let edit = spec.clone();
        let probe = spec.clone();
        let chat = spec.id.to_string();
        let status = self.status(spec);
        let connected = status.is_some_and(Status::connected);
        let busy = status.is_some_and(Status::busy);
        let has_authentication = self.authentications.contains_key(&spec.id);
        let detail = self.check_detail(spec);
        let status_content = gds_uim::el! {
            #div { flex_1; min_w_0; flex; flex_col; gap_1; text_size: $crate::ui::SMALL; } [
                ..$bool::then(!has_authentication, || gds_uim::el! {
                    #div { id: $format!("agents-status-{index}"); test_support; } [
                        $(if spec.kind == Kind::Embedded {
                            crate::i18n::tr!("agent-builtin-ready")
                        } else {
                            status.map_or(crate::i18n::tr!("agent-unchecked"), status::label)
                        })
                    ]
                }),
                ..$bool::then(!has_authentication, || gds_uim::el! {
                    #div { text_color: $cx.theme().muted_foreground; } [ $detail ]
                }),
            ]
        };
        let summary = gds_uim::el! {
            #div { flex; items_center; justify_between; gap_3; } [
                $status_content,
                ..$bool::then(spec.kind == Kind::Acp, || gds_uim::el! {
                    $button(format!("agents-check-{index}")) {
                        label: $status.map_or(crate::i18n::tr!("agent-check"), status::check_label);
                        disabled: $(busy || self.dirty(cx) || !self.registry.editable());
                        on_click: $cx.listener(move |this, _, _, cx| this.start_check(probe.clone(), true, cx));
                    }
                }),
                ..$connected.then(|| gds_uim::el! {
                    $button(format!("agents-start-chat-{index}")) {
                        label: $crate::i18n::tr!("agent-start-chat");
                        disabled: $(!self.registry.editable());
                        on_click: $cx.listener(move |this, _, _, cx| this.begin_chat(chat.clone(), cx));
                    }
                }),
                $button(gpui_kit::SharedString::from(format!("agents-edit-{index}"))) {
                    label: $crate::i18n::tr!("agent-advanced");
                    disabled: $(!self.registry.editable());
                    on_click: $cx.listener(move |this, _, window, cx| {
                        this.edit(edit.clone(), Some(edit.id.clone()), window, cx)
                    });
                },
            ]
        };
        gds_uim::el! {
            #div { flex; flex_col; gap_3; } [
                $summary,
                $self.authentication_content(spec, index, cx),
            ]
        }
        .into_any_element()
    }

    fn form_content(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let embedded = self
            .editor
            .spec()
            .is_some_and(|spec| spec.kind == Kind::Embedded);
        let connected = self.draft_connected(cx);
        let actions = gds_uim::el! {
            #div { flex; flex_wrap; gap_2; } [
                $button("agents-save") {
                    label: $(if self.editor.deleting() {
                        crate::i18n::tr!("agent-remove-confirm")
                    } else if connected {
                        crate::i18n::tr!("agent-save-config")
                    } else {
                        crate::i18n::tr!("agent-save-check")
                    });
                    on_click: $cx.listener(Self::on_save);
                },
                $button("agents-cancel") {
                    label: $crate::i18n::tr!("action-cancel-edit");
                    on_click: $cx.listener(Self::on_cancel_edit);
                },
                $button("agents-delete") {
                    label: $crate::i18n::tr!("agent-mark-remove");
                    disabled: $(self.editor.selected().is_none() || self.editor.deleting());
                    on_click: $cx.listener(Self::on_mark_delete);
                },
            ]
        };
        gds_uim::el! {
            #div { flex; flex_col; gap_4; } [
                ..$bool::then(!embedded && !connected, || gds_uim::el! {
                    #div {
                        id: "agents-connection-help"; test_support;
                        text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                    } [ $crate::i18n::tr!("agent-custom-help") ]
                }),
                ..$self.form.rows(embedded, connected).into_iter().map(|(id, label, input)| gds_uim::el! {
                    #div { flex; flex_col; gap_1; } [
                        #div { text_size: $crate::ui::SMALL; } [ $label ],
                        #div { id: $id; test_support; } [
                            Input($input) { disabled: $self.editor.deleting(); }
                        ],
                    ]
                }),
                ..$self.editor.deleting().then(|| gds_uim::el! {
                    #div { text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground; } [
                        $crate::i18n::tr!("agent-remove-confirm-help")
                    ]
                }),
                $actions,
            ]
        }.into_any_element()
    }

    fn add_content(&self, drafting: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        gds_uim::el! {
            #div { flex; flex_col; gap_4; } [
                ..$self.show_installation.then(|| self.registry_content(cx)),
                #div { flex; } [
                    $button("agents-add-custom") {
                        label: $crate::i18n::tr!("agent-add-custom");
                        disabled: $(self.document.is_none() || !self.registry.editable());
                        on_click: $cx.listener(Self::on_add_custom);
                    },
                ],
                #div {
                    flex; flex_col; gap_1; text_size: $crate::ui::SMALL;
                    text_color: $cx.theme().muted_foreground;
                } [
                    $crate::i18n::tr!("agent-custom-description"),
                    $crate::i18n::tr!("agent-auth-description"),
                ],
                ..$drafting.then(|| self.form_content(cx)),
                #div { flex; } [
                    $button("agents-installation-toggle") {
                        label: $(if self.show_installation {
                            crate::i18n::tr!("catalog-collapse")
                        } else {
                            crate::i18n::tr!("agent-install-action")
                        });
                        on_click: $cx.listener(Self::on_toggle_installation);
                    },
                ],
            ]
        }
        .into_any_element()
    }

    fn on_save(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn on_cancel_edit(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.editor = Editor::Idle;
        self.message = crate::i18n::tr!("agent-edit-cancelled").into();
        cx.notify();
    }

    fn on_mark_delete(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.mark_delete();
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

    fn on_toggle_installation(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_installation = !self.show_installation;
        cx.notify();
    }

    fn on_open_installation(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.ensure_authentication_panels(window, cx);
        let dirty = self.dirty(cx);
        let catalog = self.display_catalog();
        let drafting = matches!(self.editor, Editor::Add(_));
        let section_count = if self.show_installation {
            2
        } else {
            catalog.len() + 1
        };
        if let Some(index) = self.scroll_to.take() {
            self.scroll
                .scroll_to_top_of_item(index.min(section_count.saturating_sub(1)));
        }
        let header = if self.show_installation {
            self.registry_header(cx)
        } else {
            gds_uim::el! {
                #div { flex; flex_col; gap_2; } [
                    #div { flex; items_center; justify_between; gap_3; } [
                        #div { text_size: $crate::ui::TITLE; } [
                            $crate::i18n::tr!("agent-advanced-title")
                        ],
                        $button("agents-installation-help") {
                            label: $crate::i18n::tr!("agent-install-action");
                            on_click: $cx.listener(Self::on_open_installation);
                        },
                        $button("agents-reload") {
                            label: $crate::i18n::tr!("settings-reload");
                            on_click: $cx.listener(Self::on_reload_agents);
                        },
                    ],
                    #div {
                        text_size: $crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
                    } [
                        $crate::i18n::tr!(
                            "settings-file",
                            path = (self.store.path().display()).to_string()
                        )
                    ],
                ]
            }
            .into_any_element()
        };
        let mut content = Vec::new();
        if self.show_installation {
            content.push(self.registry_content(cx));
            content.push(
                gds_uim::el! {
                    #div { flex; justify_end; } [
                        Button("agents-add-custom") {
                            ghost; small; label: $crate::i18n::tr!("agent-add-custom");
                            disabled: $(self.document.is_none() || !self.registry.editable());
                            on_click: $cx.listener(Self::on_add_custom);
                        },
                    ]
                }
                .into_any_element(),
            );
        } else {
            for (index, spec) in catalog.iter().enumerate() {
                let body = if self.editor.selected() == Some(&spec.id) {
                    self.form_content(cx)
                } else {
                    self.summary_content(index, spec, cx)
                };
                let entry = self.registry.catalog.as_ref().and_then(|catalog| {
                    catalog
                        .entries
                        .iter()
                        .find(|entry| entry.spec.as_ref() == Some(spec))
                });
                let title = entry.map_or_else(|| spec.id.to_string(), |entry| entry.name.clone());
                let description = entry
                    .filter(|entry| entry.source == bas::management::Source::Registry)
                    .map_or_else(
                        || summary(spec),
                        |entry| {
                            format!(
                                "{} · ACP Registry · {}",
                                entry.description,
                                entry.installed_version.as_deref().unwrap_or(&entry.version)
                            )
                        },
                    );
                content.push(self.card(
                    format!("agents-card-{index}"),
                    title,
                    description,
                    body,
                    cx,
                ));
            }
            let add = self.add_content(drafting, cx);
            content.push(self.card(
                "agents-card-add".to_owned(),
                crate::i18n::tr!("agent-add-custom").to_owned(),
                crate::i18n::tr!("agent-custom-description").to_owned(),
                add,
                cx,
            ));
        }
        let heading = gds_uim::el! {
            #div { p_5; flex_shrink_0; flex; flex_col; gap_2; } [
                $header,
                ..$dirty.then(|| gds_uim::el! {
                    #div { id: "agents-dirty"; test_support; text_size: $crate::ui::SMALL; } [
                        $crate::i18n::tr!("settings-unsaved")
                    ]
                }),
                ..$bool::then(!self.message.is_empty(), || gds_uim::el! {
                    #div {
                        id: "agents-message"; test_support; text_size: $crate::ui::SMALL;
                        text_color: $cx.theme().muted_foreground;
                    } [ $self.message.clone() ]
                }),
                ..$bool::then(!self.registration_errors.is_empty(), || gds_uim::el! {
                    #div { id: "agents-registration-errors"; test_support; text_size: $crate::ui::SMALL; } [
                        $self.registration_errors.clone()
                    ]
                }),
            ]
        };
        gds_uim::el! {
            #div {
                id: "agents-page"; test_support; size_full; flex; flex_col;
                bg: $registry::Palette::current(cx).background;
            } [
                $heading,
                #div {
                    id: "agents-sections"; test_support; flex_1; min_h_0; overflow_y_scroll;
                    track_scroll: $&self.scroll; px_5; pb_5; flex; flex_col; gap_4;
                } [ ..content ],
            ]
        }
    }
}
