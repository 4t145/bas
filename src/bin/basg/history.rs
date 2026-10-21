//! Independently owned Agent history browser. Discovery never creates or prompts a session.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::PathBuf,
    rc::Rc,
};

use bas::{
    acp,
    model::{SessionId, SessionSnapshot, SessionStatus},
};
use gpui_kit::{
    App, AppContext, Context, Entity, Subscription, Task, Window,
    component::input::{InputEvent, InputState},
};

use crate::connection::{self, AgentSummary, Command, Connection, Update};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
mod view;
pub(crate) mod window;

const MAX_DISCOVERIES: usize = 2;
pub(crate) const ICON: gpui_kit::assets::IconName = gpui_kit::assets::IconName::MessagesSquare;

pub(crate) type OpenSession = Rc<dyn Fn(SessionSnapshot, &mut App) -> bool>;

#[derive(Clone)]
pub(crate) struct OpenRequest {
    pub server: bas::service::tls::ConnectionOptions,
    pub agents: Vec<AgentSummary>,
    pub agent: Option<String>,
    pub cwd: Option<PathBuf>,
    pub workspaces: Vec<PathBuf>,
    pub open_session: OpenSession,
}

struct Query {
    generation: u64,
    agent: String,
}

#[derive(Clone)]
struct Target {
    generation: u64,
    agent: String,
    info: acp::SessionInfo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PrimaryAction {
    Open,
    Restart,
    Restore,
}

enum Operation {
    Import { request: u64, target: Target },
    Delete { request: u64, target: Target },
    Close { session: SessionId },
    Restart { session: SessionId, generation: u64 },
    Restore { session: SessionId, generation: u64 },
}

pub(crate) struct Browser {
    server: bas::service::tls::ConnectionOptions,
    connection: Connection,
    agents: Vec<AgentSummary>,
    awaiting_catalog: bool,
    agent: Option<String>,
    cwd: Option<PathBuf>,
    workspaces: BTreeSet<PathBuf>,
    pages: BTreeMap<String, connection::AgentHistory>,
    sessions: HashMap<SessionId, SessionSnapshot>,
    observed_sessions: HashSet<SessionId>,
    queries: HashMap<u64, Query>,
    waiting: VecDeque<String>,
    generation: u64,
    next_request: u64,
    operation: Option<Operation>,
    errors: BTreeMap<String, String>,
    connection_error: Option<String>,
    notice: Option<String>,
    search: Entity<InputState>,
    open_session: OpenSession,
    _search_subscription: Subscription,
    _events: Task<()>,
    _clock: Task<()>,
}

impl Browser {
    fn new(
        request: OpenRequest,
        connection: Connection,
        mut events: tokio::sync::mpsc::Receiver<Update>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::i18n::tr!("agent-history-search"))
        });
        let subscription = cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let events = cx.spawn(async move |view, cx| {
            while let Some(update) = events.recv().await {
                if view.update(cx, |this, cx| this.update(update, cx)).is_err() {
                    break;
                }
            }
        });
        let clock = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(crate::ui::time::REFRESH_INTERVAL)
                    .await;
                if view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut browser = Self {
            server: request.server,
            connection,
            agents: Vec::new(),
            awaiting_catalog: request.agents.is_empty(),
            agent: request.agent,
            cwd: request.cwd,
            workspaces: request.workspaces.into_iter().collect(),
            pages: BTreeMap::new(),
            sessions: HashMap::new(),
            observed_sessions: HashSet::new(),
            queries: HashMap::new(),
            waiting: VecDeque::new(),
            generation: 0,
            next_request: 0,
            operation: None,
            errors: BTreeMap::new(),
            connection_error: None,
            notice: None,
            search,
            open_session: request.open_session,
            _search_subscription: subscription,
            _events: events,
            _clock: clock,
        };
        browser.set_agents(request.agents);
        if let Some(cwd) = &browser.cwd {
            browser.workspaces.insert(cwd.clone());
        }
        browser.reload(cx);
        browser
    }

    fn set_agents(&mut self, agents: Vec<AgentSummary>) {
        self.agents = agents
            .into_iter()
            .filter(|agent| agent.kind == "acp")
            .collect();
        self.agents.sort_by(|a, b| a.id.cmp(&b.id));
        self.agents.dedup_by(|a, b| a.id == b.id);
        if self
            .agent
            .as_ref()
            .is_some_and(|selected| !self.agents.iter().any(|agent| &agent.id == selected))
        {
            self.agent = None;
        }
    }

    fn configure(&mut self, request: OpenRequest, cx: &mut Context<Self>) {
        // Reopening the window cannot retarget an operation awaiting a response.
        if self.operation.is_some() {
            return;
        }
        self.agent = request.agent;
        self.cwd = request.cwd;
        self.workspaces.extend(request.workspaces);
        if let Some(cwd) = &self.cwd {
            self.workspaces.insert(cwd.clone());
        }
        self.open_session = request.open_session;
        self.set_agents(request.agents);
        self.reload(cx);
    }

    fn select_agent(&mut self, agent: Option<String>, cx: &mut Context<Self>) {
        if self.agent == agent {
            return;
        }
        self.agent = agent;
        self.reload(cx);
    }

    fn select_workspace(&mut self, cwd: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.cwd == cwd {
            return;
        }
        self.cwd = cwd;
        self.reload(cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.pages.clear();
        self.errors.clear();
        self.notice = None;
        self.waiting = self
            .agents
            .iter()
            .filter(|agent| {
                self.agent
                    .as_ref()
                    .is_none_or(|selected| selected == &agent.id)
            })
            .map(|agent| agent.id.clone())
            .collect();
        // Retain old in-flight requests until they settle, so rapid filter changes
        // cannot spawn an unbounded number of Agent processes.
        self.pump(cx);
        cx.notify();
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.queries.len() < MAX_DISCOVERIES {
            let Some(agent) = self.waiting.pop_front() else {
                break;
            };
            let request = self.request_id();
            let command = Command::AgentHistory {
                epoch: request,
                agent: agent.clone(),
                cwd: self.cwd.clone(),
            };
            if self.connection.send(command).is_err() {
                self.connection_error = Some(crate::i18n::tr!("connection-reopen").into());
                self.waiting.clear();
                break;
            }
            self.queries.insert(
                request,
                Query {
                    generation: self.generation,
                    agent,
                },
            );
        }
        cx.notify();
    }

    fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    fn loading(&self) -> bool {
        self.awaiting_catalog
            || !self.waiting.is_empty()
            || self
                .queries
                .values()
                .any(|query| query.generation == self.generation)
    }

    fn update(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Catalog(Ok(catalog)) => {
                self.awaiting_catalog = false;
                self.connection_error = None;
                let previous = self.agents.clone();
                self.set_agents(catalog.agents);
                let ids: HashSet<_> = catalog
                    .sessions
                    .iter()
                    .map(|snapshot| snapshot.meta.id)
                    .collect();
                // Only disappearance after a catalog observation proves removal:
                // an import receipt may overtake a catalog started before the import.
                let removed: HashSet<_> =
                    self.observed_sessions.difference(&ids).copied().collect();
                self.sessions.retain(|id, _| !removed.contains(id));
                self.observed_sessions = ids;
                for snapshot in catalog.sessions {
                    if let Some(cwd) = &snapshot.meta.cwd {
                        self.workspaces.insert(cwd.clone());
                    }
                    self.merge_session(snapshot);
                }
                if previous != self.agents {
                    self.reload(cx);
                }
            }
            Update::Catalog(Err(error)) => {
                self.awaiting_catalog = false;
                self.connection_error = Some(error.presentation());
            }
            Update::AgentHistory { epoch, result } => {
                if let Some(query) = self.queries.remove(&epoch)
                    && query.generation == self.generation
                {
                    match result {
                        Ok(page) => {
                            for info in &page.sessions.sessions {
                                self.workspaces.insert(info.cwd.clone());
                            }
                            self.pages.insert(query.agent, page);
                        }
                        Err(error) => {
                            self.errors.insert(query.agent, error.presentation());
                        }
                    }
                }
                self.pump(cx);
            }
            Update::ImportedHistory { epoch, result } => {
                if matches!(&self.operation, Some(Operation::Import { request, .. }) if *request == epoch)
                {
                    let Some(Operation::Import { target, .. }) = self.operation.take() else {
                        unreachable!()
                    };
                    match result {
                        Ok(snapshot) => {
                            let id = snapshot.meta.id;
                            self.merge_session(snapshot.clone());
                            if target.generation == self.generation {
                                if let Some(page) = self.pages.get_mut(&target.agent) {
                                    page.imported
                                        .insert(target.info.session_id.to_string(), id.to_string());
                                }
                                self.open_record(snapshot, cx);
                            }
                        }
                        Err(error) if target.generation == self.generation => {
                            self.errors.insert(target.agent, error.presentation());
                        }
                        Err(_) => {}
                    }
                }
            }
            Update::DeletedAgentHistory { epoch, id, result } => {
                if matches!(&self.operation, Some(Operation::Delete { request, target }) if *request == epoch && target.info.session_id.to_string() == id)
                {
                    let Some(Operation::Delete { target, .. }) = self.operation.take() else {
                        unreachable!()
                    };
                    if target.generation == self.generation {
                        match result {
                            Ok(()) => {
                                if let Some(page) = self.pages.get_mut(&target.agent) {
                                    page.sessions
                                        .sessions
                                        .retain(|info| info.session_id.to_string() != id);
                                }
                            }
                            Err(error) => {
                                self.errors.insert(target.agent, error.presentation());
                            }
                        }
                    }
                }
            }
            Update::RestartedSession { session, result } if matches!(&self.operation, Some(Operation::Restart { session: pending, .. }) if *pending == session) =>
            {
                let Some(Operation::Restart { generation, .. }) = self.operation.take() else {
                    unreachable!()
                };
                self.complete_open_operation(result, generation, cx);
            }
            Update::Archived { session, result } if matches!(&self.operation, Some(Operation::Restore { session: pending, .. }) if *pending == session) =>
            {
                let Some(Operation::Restore { generation, .. }) = self.operation.take() else {
                    unreachable!()
                };
                self.complete_open_operation(result, generation, cx);
            }
            Update::ClosedSession { session, result } => {
                if matches!(&self.operation, Some(Operation::Close { session: pending }) if *pending == session)
                {
                    self.operation = None;
                    match result {
                        Ok(snapshot) => self.merge_session(snapshot),
                        Err(error) => self.notice = Some(error.presentation()),
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn merge_session(&mut self, snapshot: SessionSnapshot) {
        let id = snapshot.meta.id;
        if self
            .sessions
            .get(&id)
            .is_none_or(|old| old.meta.revision <= snapshot.meta.revision)
        {
            self.sessions.insert(id, snapshot);
        }
    }

    fn imported(&self, agent: &str, info: &acp::SessionInfo) -> Option<&SessionSnapshot> {
        self.pages
            .get(agent)?
            .imported
            .get(&info.session_id.to_string())
            .and_then(|id| id.parse().ok())
            .and_then(|id| self.sessions.get(&id))
    }

    fn current(&self, target: &Target) -> bool {
        target.generation == self.generation
            && self
                .agent
                .as_ref()
                .is_none_or(|agent| agent == &target.agent)
            && self.cwd.as_ref().is_none_or(|cwd| cwd == &target.info.cwd)
            && self.pages.get(&target.agent).is_some_and(|page| {
                page.sessions.sessions.iter().any(|info| {
                    info.session_id == target.info.session_id && info.cwd == target.info.cwd
                })
            })
    }

    fn primary_action(&self, agent: &str, info: &acp::SessionInfo) -> PrimaryAction {
        match self.imported(agent, info) {
            Some(session) if session.meta.archived => PrimaryAction::Restore,
            Some(session) if session.meta.status == SessionStatus::Stopped => {
                PrimaryAction::Restart
            }
            _ => PrimaryAction::Open,
        }
    }

    fn activate(&mut self, target: Target, expected: PrimaryAction, cx: &mut Context<Self>) {
        if !self.current(&target) || self.primary_action(&target.agent, &target.info) != expected {
            return;
        }
        match expected {
            PrimaryAction::Open => self.import(target, cx),
            PrimaryAction::Restart => self.restart(target, cx),
            PrimaryAction::Restore => self.restore(target, cx),
        }
    }

    fn import(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.current(&target) || self.operation.is_some() {
            return;
        }
        if let Some(snapshot) = self.imported(&target.agent, &target.info).cloned() {
            self.open_record(snapshot, cx);
            return;
        }
        let capabilities = &self.pages[&target.agent].initialization.agent_capabilities;
        if !target.info.additional_directories.is_empty()
            || !(capabilities.load_session || capabilities.session_capabilities.resume.is_some())
        {
            return;
        }
        let request = self.request_id();
        if self.send(
            Command::ImportHistory {
                epoch: request,
                agent: target.agent.clone(),
                info: target.info.clone(),
            },
            cx,
        ) {
            self.operation = Some(Operation::Import { request, target });
        }
    }

    fn open_record(&mut self, snapshot: SessionSnapshot, cx: &mut Context<Self>) {
        if !(self.open_session)(snapshot, cx) {
            self.notice = Some(crate::i18n::tr!("agent-history-imported").into());
        }
    }

    fn complete_open_operation(
        &mut self,
        result: Result<SessionSnapshot, connection::Failure>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(snapshot) => {
                self.merge_session(snapshot.clone());
                if generation == self.generation {
                    self.open_record(snapshot, cx);
                }
            }
            Err(error) => self.notice = Some(error.presentation()),
        }
    }

    fn restart(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.current(&target) || self.operation.is_some() {
            return;
        }
        let Some(snapshot) = self.imported(&target.agent, &target.info) else {
            return;
        };
        if snapshot.meta.status != SessionStatus::Stopped || snapshot.meta.archived {
            return;
        }
        let capabilities = &self.pages[&target.agent].initialization.agent_capabilities;
        if !(capabilities.load_session || capabilities.session_capabilities.resume.is_some()) {
            return;
        }
        let session = snapshot.meta.id;
        if self.send(Command::RestartSession(session), cx) {
            self.operation = Some(Operation::Restart {
                session,
                generation: self.generation,
            });
        }
    }

    fn restore(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.current(&target) || self.operation.is_some() {
            return;
        }
        let Some(snapshot) = self.imported(&target.agent, &target.info) else {
            return;
        };
        if !snapshot.meta.archived {
            return;
        }
        let session = snapshot.meta.id;
        if self.send(
            Command::Archive {
                session,
                archived: false,
            },
            cx,
        ) {
            self.operation = Some(Operation::Restore {
                session,
                generation: self.generation,
            });
        }
    }

    fn active(&self, target: &Target) -> bool {
        self.imported(&target.agent, &target.info)
            .is_some_and(|snapshot| {
                !snapshot.meta.archived
                    && !matches!(
                        snapshot.meta.status,
                        SessionStatus::Stopped | SessionStatus::Failed
                    )
            })
    }

    fn delete(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.current(&target) || self.operation.is_some() || self.active(&target) {
            return;
        }
        if self.pages[&target.agent]
            .initialization
            .agent_capabilities
            .session_capabilities
            .delete
            .is_none()
        {
            return;
        }
        let confirmation = crate::application::Confirmation {
            title: crate::i18n::tr!("agent-history-delete").into(),
            message: crate::i18n::tr!(
                "agent-history-delete-confirm",
                title = target
                    .info
                    .title
                    .as_deref()
                    .unwrap_or(crate::i18n::tr!("agent-history-untitled"))
            ),
            confirm_label: crate::i18n::tr!("agent-history-delete").into(),
            destructive: true,
        };
        let owner = cx.entity().downgrade();
        if let Err(error) = crate::application::open_delete_confirmation(
            confirmation,
            move |cx| {
                let _ = owner.update(cx, |this, cx| {
                    if !this.current(&target) || this.operation.is_some() || this.active(&target) {
                        return;
                    }
                    let request = this.request_id();
                    if this.send(
                        Command::DeleteAgentHistory {
                            epoch: request,
                            agent: target.agent.clone(),
                            id: target.info.session_id.to_string(),
                        },
                        cx,
                    ) {
                        this.operation = Some(Operation::Delete { request, target });
                    }
                });
            },
            cx,
        ) {
            self.notice = Some(error.to_string());
        }
    }

    fn close(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.current(&target) || self.operation.is_some() || !self.active(&target) {
            return;
        }
        let Some(session) = self
            .imported(&target.agent, &target.info)
            .map(|snapshot| snapshot.meta.id)
        else {
            return;
        };
        if self.send(Command::CloseSession(session), cx) {
            self.operation = Some(Operation::Close { session });
        }
    }

    fn send(&mut self, command: Command, cx: &mut Context<Self>) -> bool {
        self.notice = None;
        let sent = self.connection.send(command).is_ok();
        if !sent {
            self.connection_error = Some(crate::i18n::tr!("connection-reopen").into());
        }
        cx.notify();
        sent
    }
}
