mod application;
mod archive;
mod attachments;
mod chat;
mod commands;
mod composer;
mod forms;
mod groups;
mod input_layout;
mod layout;
mod markdown;
mod menus;
mod message_actions;
mod navigation;
mod outbox;
#[cfg(feature = "gui-tests")]
mod preview;
mod queue;
mod reply;
mod restore;
mod settings;
mod sidebar;
mod status;
mod thought;
mod thread_actions;
mod thread_row;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
};

use basr::{
    model::{SessionId, SessionMeta, SessionStatus},
    service::pb,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::InputEvent;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Div, Entity, FollowMode, FontWeight, KeyBinding,
    ListAlignment, ListState, Pixels, SharedString, Size, Subscription, Task, TestSupportExt,
    Window, actions,
    base::TextView,
    component::{button::*, input::*, *},
    div, list,
    prelude::*,
    px, size,
};

use crate::{
    attachment::{Attachment, Draft},
    connection::{self, Command, Connection, StreamState, Update},
    conversation::{Conversation, Item},
    desktop, theme,
};

const EVENT_BATCH: usize = 64;
const LIST_OVERDRAW: f32 = 800.;
const SIDEBAR_WIDTH: f32 = 252.;
const AGENT_ICON_SIZE: f32 = 16.;

use crate::application::OpenSettings;
pub(super) use layout::initial_size;

actions!(
    basr,
    [
        NewSession,
        FocusSearch,
        FocusComposer,
        ToggleSidebar,
        ToggleMode,
        ToggleOutput,
        NextThread,
        PreviousThread,
        Dismiss,
        Refresh
    ]
);

pub(super) fn bind_keys(cx: &mut App) {
    composer::bind_keys(cx);
    cx.bind_keys([
        KeyBinding::new("ctrl-n", NewSession, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-f", FocusSearch, Some("BasrWorkspace")),
        // Override Input's own search action while editing a BASR field.
        KeyBinding::new("ctrl-f", FocusSearch, Some("BasrWorkspace > Input")),
        KeyBinding::new("ctrl-l", FocusComposer, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-b", ToggleSidebar, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-shift-m", ToggleMode, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-o", ToggleOutput, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-tab", NextThread, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousThread, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-tab", NextThread, Some("BasrWorkspace > Input")),
        KeyBinding::new(
            "ctrl-shift-tab",
            PreviousThread,
            Some("BasrWorkspace > Input"),
        ),
        KeyBinding::new("escape", Dismiss, Some("BasrWorkspace")),
        KeyBinding::new("f5", Refresh, Some("BasrWorkspace")),
    ]);
}

pub(super) struct Workspace {
    window: gpui_kit::AnyWindowHandle,
    menu: Option<menus::OpenMenu>,
    reply_preview: Option<reply::Preview>,
    reply_hover: Option<Task<()>>,
    seen_replies: HashMap<SessionId, u64>,
    pinned: HashSet<SessionId>,
    foreground: Option<SessionId>,
    connection: Connection,
    restore_store: Option<restore::Store>,
    restore_on_connect: bool,
    server: SocketAddr,
    sessions: Vec<SessionMeta>,
    catalog_ids: HashSet<SessionId>,
    removed_sessions: HashSet<SessionId>,
    agents: Vec<pb::Agent>,
    agent_metadata: Option<Entity<crate::agent_metadata::Metadata>>,
    active: Option<SessionId>,
    conversation: Option<Conversation>,
    epoch: u64,
    stream: StreamState,
    connected: bool,
    connection_error: Option<String>,
    error: Option<String>,
    authentication_required: Option<String>,
    authentication_panel: Option<Entity<crate::authentication::Authentication>>,
    authentication_load: Option<Task<()>>,
    elicitation_panels:
        std::collections::BTreeMap<String, Entity<crate::authentication::elicitation::Panel>>,
    search: Entity<InputState>,
    composer: Entity<EditorState>,
    composer_expanded: bool,
    composer_available_height: Cell<f32>,
    composer_mode: composer::Mode,
    approvals: crate::approval::Panel,
    approval_error: Option<String>,
    composer_layout: input_layout::InputLayout,
    composer_rows: usize,
    composer_content_left: Cell<Pixels>,
    mention_decorations: TextDecorationCollection,
    decorated_mentions: Vec<TextDecoration>,
    mention_selection: std::ops::Range<usize>,
    cwd: Entity<InputState>,
    rename: Entity<InputState>,
    selected_agent: String,
    unstarted_sessions: HashSet<SessionId>,
    prompted_sessions: HashSet<SessionId>,
    drafts: HashMap<SessionId, Draft>,
    attachments: Vec<Attachment>,
    loading_attachments: HashSet<SessionId>,
    paste_pending: bool,
    composer_generation: u64,
    new_drafts: HashMap<Option<PathBuf>, Draft>,
    directories: Vec<PathBuf>,
    selected_directory: Option<PathBuf>,
    collapsed_workspaces: HashSet<Option<PathBuf>>,
    command_selection: usize,
    commands_dismissed: bool,
    markdown: RefCell<HashMap<String, markdown::Document>>,
    outboxes: HashMap<SessionId, outbox::SessionOutbox>,
    configuring: HashSet<SessionId>,
    cancelling: HashSet<SessionId>,
    creating: bool,
    waiting_for_agent_selection: bool,
    editing_title: bool,
    renaming: bool,
    archived_list: bool,
    archive_pending: HashSet<SessionId>,
    delete_pending: HashSet<SessionId>,
    sidebar_visible: bool,
    compact: bool,
    output_open: bool,
    editing_workspace: bool,
    window_size: Size<Pixels>,
    popup_size: Option<Size<Pixels>>,
    placement: Option<desktop::Placement>,
    expanded: HashSet<String>,
    list: ListState,
    _subscriptions: Vec<Subscription>,
    _events: Task<()>,
}

impl Workspace {
    pub fn new(
        server: SocketAddr,
        cwd: PathBuf,
        compact: bool,
        runtime: &tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (connection, events) = Connection::new(server, runtime);
        let mut view = Self::with_connection(connection, events, server, cwd, compact, window, cx);
        view.restore_store = restore::Store::discover(server);
        view.pinned = view
            .restore_store
            .as_ref()
            .map(restore::Store::pinned)
            .unwrap_or_default();
        view.restore_on_connect = true;
        view.placement = Some(desktop::Placement::new(runtime));
        view.resize_window(window, cx);
        view
    }

    fn with_connection(
        connection: Connection,
        mut events: tokio::sync::mpsc::Receiver<Update>,
        server: SocketAddr,
        cwd: PathBuf,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = view.update(cx, |this, cx| {
                this.prepare_to_close(window, cx);
            });
            true
        });
        let selected_directory = Some(groups::normalize(&cwd));
        let directories = vec![groups::normalize(&cwd)];
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索 threads…"));
        let composer_mode = if compact {
            cx.global::<theme::Appearance>().composer_enter_behavior
        } else {
            composer::Mode::Newline
        };
        let composer = cx.new(|cx| {
            EditorState::new(window, cx)
                .placeholder("描述你想完成的任务…")
                .language("markdown")
                .line_number(false)
                .folding(false)
                .auto_close(false)
                .smart_indent(false)
                .soft_wrap(true)
                .scroll_beyond_last_line(Some(0))
                .submit_on_enter(composer_mode == composer::Mode::Send)
        });
        composer.update(cx, |input, cx| input.focus(window, cx));
        let mention_decorations = composer.update(cx, |input, cx| {
            input.create_decorations_collection(Vec::new(), cx)
        });
        let cwd = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Agent 的工作目录");
            input.set_value(cwd.to_string_lossy().to_string(), window, cx);
            input
        });
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("会话标题"));
        if cx.try_global::<crate::settings::AgentChanges>().is_none() {
            cx.set_global(crate::settings::AgentChanges::default());
        }
        crate::agent_metadata::ensure(server, cx);
        let agent_metadata = crate::agent_metadata::entity(cx);
        let mut subscriptions = vec![
            cx.observe_global_in::<crate::settings::AgentChanges>(window, |this, window, cx| {
                let changes = cx.global::<crate::settings::AgentChanges>().clone();
                if changes.server != Some(this.server) {
                    return;
                }
                this.command(Command::Refresh);
                if changes.target == Some(this.window.window_id())
                    && let Some(agent) = changes.select
                {
                    this.start_agent_chat(agent, window, cx);
                    window.activate_window();
                    return;
                }
                if changes
                    .ready
                    .as_ref()
                    .is_some_and(|agent| this.authentication_required.as_ref() == Some(agent))
                {
                    this.refresh(cx);
                }
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.sync_foreground(window);
                if !window.is_window_active() {
                    this.close_reply_preview(window, cx);
                }
                cx.notify();
            }),
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.subscribe_in(&search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let query = this.search.read(cx).value().to_lowercase();
                    if let Some(id) = this
                        .visible_sessions()
                        .iter()
                        .find(|session| matches_query(session, &query))
                        .map(|session| session.id)
                    {
                        this.select(id, window, cx);
                    }
                }
            }),
            cx.observe_global::<Theme>(|this, cx| {
                this.sync_mentions(cx);
                this.list.remeasure_items(
                    0..this
                        .conversation
                        .as_ref()
                        .map_or(0, |state| state.rows.len()),
                );
                cx.notify();
            }),
            cx.observe_global::<theme::Appearance>(|this, cx| this.sync_composer_mode(cx)),
            cx.observe(&composer, |this, _, cx| {
                this.sync_mentions(cx);
                cx.notify();
            }),
            cx.observe(&rename, |_, _, cx| cx.notify()),
            cx.subscribe_in(&rename, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_title(cx);
                }
            }),
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                if this.composer_mode == composer::Mode::Send
                    && matches!(event, InputEvent::PressEnter { shift: false, .. })
                    && !this.complete_command(window, cx)
                {
                    this.send_prompt(window, cx);
                }
                if matches!(event, InputEvent::Change) {
                    this.error = None;
                    this.command_selection = 0;
                    this.commands_dismissed = false;
                    if this.compact {
                        this.resize_window(window, cx);
                    }
                }
                cx.notify();
            }),
        ];
        if let Some(metadata) = &agent_metadata {
            subscriptions.push(cx.observe(metadata, |_, _, cx| cx.notify()));
        }
        let event_task = cx.spawn_in(window, async move |view, cx| {
            while let Some(first) = events.recv().await {
                let mut batch = vec![first];
                while batch.len() < EVENT_BATCH {
                    let Ok(event) = events.try_recv() else { break };
                    batch.push(event);
                }
                if view
                    .update_in(cx, |this, window, cx| {
                        for update in batch {
                            this.update(update, window, cx);
                        }
                        if this.compact {
                            this.resize_window(window, cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let list = ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW));
        let view = cx.entity().downgrade();
        list.set_scroll_handler(move |_, _, cx| {
            let _ = view.update(cx, |_, cx| cx.notify());
        });
        list.set_follow_mode(FollowMode::Tail);
        Self {
            connection,
            restore_store: None,
            restore_on_connect: false,
            server,
            sessions: Vec::new(),
            catalog_ids: HashSet::new(),
            removed_sessions: HashSet::new(),
            agents: Vec::new(),
            agent_metadata,
            active: None,
            conversation: None,
            epoch: 0,
            stream: StreamState::Loading,
            connected: false,
            connection_error: None,
            error: None,
            authentication_required: None,
            authentication_panel: None,
            authentication_load: None,
            elicitation_panels: std::collections::BTreeMap::new(),
            search,
            composer,
            composer_expanded: !compact,
            composer_available_height: Cell::new(620.),
            composer_mode,
            approvals: crate::approval::Panel::default(),
            approval_error: None,
            composer_layout: input_layout::InputLayout::new(
                gpui_kit::font(cx.theme().font_family.clone()),
                theme::content_font_size(cx),
            ),
            composer_rows: 1,
            composer_content_left: Cell::new(gpui_kit::component::Size::Medium.input_px()),
            mention_decorations,
            decorated_mentions: Vec::new(),
            mention_selection: 0..0,
            cwd,
            rename,
            selected_agent: String::new(),
            unstarted_sessions: HashSet::new(),
            prompted_sessions: HashSet::new(),
            drafts: HashMap::new(),
            attachments: Vec::new(),
            loading_attachments: HashSet::new(),
            paste_pending: false,
            composer_generation: 0,
            new_drafts: HashMap::new(),
            selected_directory,
            directories,
            collapsed_workspaces: HashSet::new(),
            command_selection: 0,
            commands_dismissed: false,
            markdown: RefCell::new(HashMap::new()),
            outboxes: HashMap::new(),
            configuring: HashSet::new(),
            cancelling: HashSet::new(),
            creating: false,
            waiting_for_agent_selection: false,
            editing_title: false,
            renaming: false,
            archived_list: false,
            archive_pending: HashSet::new(),
            delete_pending: HashSet::new(),
            window: window.window_handle(),
            menu: None,
            reply_preview: None,
            reply_hover: None,
            seen_replies: HashMap::new(),
            pinned: HashSet::new(),
            foreground: None,
            sidebar_visible: !compact,
            compact,
            output_open: false,
            editing_workspace: false,
            window_size: initial_size(false),
            popup_size: None,
            placement: None,
            expanded: HashSet::new(),
            list,
            _subscriptions: subscriptions,
            _events: event_task,
        }
    }

    fn command(&mut self, command: Command) {
        self.error = None;
        let prompt = match &command {
            Command::Prompt { session, .. } => Some(*session),
            _ => None,
        };
        let permission = match &command {
            Command::RespondPermission {
                session,
                request_id,
                ..
            } => Some((*session, request_id.clone())),
            _ => None,
        };
        if !self.connection.send(command) {
            if let Some((session, request_id)) = permission
                && self.active == Some(session)
            {
                self.approvals
                    .responded(&request_id, Err("连接任务已停止，请重新打开 BASR。".into()));
            }
            if let Some(session) = prompt
                && let Some(outbox) = self.outboxes.get_mut(&session)
                && let Some(outbox::Action::Rejected {
                    origin: outbox::Origin::Composer,
                    draft,
                }) = outbox.receipt(None)
            {
                self.drafts.insert(session, draft);
            }
            self.connected = false;
            self.connection_error = Some("连接任务已停止，请重新打开 BASR。".into());
        }
    }

    fn sync_foreground(&mut self, window: &Window) {
        let foreground = self.active.filter(|_| window.is_window_active());
        if self.foreground != foreground {
            self.foreground = foreground;
            self.connection.send(Command::Foreground(foreground));
        }
    }

    fn composer_error(&self, cx: &App) -> Option<String> {
        self.error
            .clone()
            .or_else(|| self.approval_error.clone())
            .or_else(|| self.authentication_hint())
            .or_else(|| self.connection_error.clone())
            .or_else(|| match &self.stream {
                StreamState::Reconnecting(error) | StreamState::Unavailable(error) => {
                    Some(error.clone())
                }
                _ => None,
            })
            .or_else(|| self.attachment_error(cx))
    }

    fn send_hint(&self, cx: &App) -> String {
        let hint = if self.paste_pending {
            "正在读取剪贴板…"
        } else if self.connection_status() != status::ConnectionStatus::Connected {
            "连接未就绪，暂不能发送"
        } else if self.creating {
            "正在创建 thread，暂不能发送"
        } else if self.waiting_for_agent_selection {
            "请先选择 Agent"
        } else if self.active.is_some_and(|id| self.archive_blocked(id)) {
            "已归档或正在更新归档状态；恢复后可发送，待发送队列需手动重试"
        } else if self.active.is_some_and(|id| self.sending(id)) {
            "正在发送…"
        } else if self.working() {
            "加入待发送队列"
        } else {
            "发送"
        };
        let hint = if self.working() {
            format!(
                "{hint} · {}\n当前回复结束后发送 · {} 换行",
                self.composer_send_shortcut(),
                self.composer_newline_shortcut()
            )
        } else {
            format!(
                "{hint} · {}\n换行 · {}",
                self.composer_send_shortcut(),
                self.composer_newline_shortcut()
            )
        };
        self.composer_error(cx)
            .map(|error| format!("{error}\n\n{hint}"))
            .unwrap_or(hint)
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.command(Command::Refresh);
        if self.active.is_none() {
            if self.authentication_required.take().is_some() {
                self.epoch += 1;
                self.creating = false;
                self.authentication_panel = None;
                self.authentication_load = None;
            }
            self.error = None;
            self.prepare_session(cx);
        }
        if let Some(session) = self.active {
            self.epoch += 1;
            self.stream = StreamState::Loading;
            self.command(Command::Watch {
                session,
                epoch: self.epoch,
            });
            cx.notify();
        }
    }

    fn save_draft(&mut self, cx: &App) {
        let draft = Draft {
            text: self.composer.read(cx).value().to_string(),
            attachments: self.attachments.clone(),
        };
        if let Some(id) = self.active {
            self.drafts.insert(id, draft);
        } else {
            self.new_drafts
                .insert(self.selected_directory.clone(), draft);
        }
    }

    fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.compact {
            self.toggle_mode(window, cx);
            self.sidebar_visible = true;
        } else {
            self.sidebar_visible = !self.sidebar_visible;
        }
        self.focus_input(window, cx);
        cx.notify();
    }

    fn select(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(id, true, window, cx);
    }

    fn activate(
        &mut self,
        id: SessionId,
        restore_draft: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active == Some(id) {
            return;
        }
        self.save_draft(cx);
        self.close_reply_preview(window, cx);
        self.active = Some(id);
        self.waiting_for_agent_selection = false;
        self.restore_on_connect = false;
        self.remember_thread();
        self.creating = false;
        self.authentication_required = None;
        self.authentication_panel = None;
        self.authentication_load = None;
        self.elicitation_panels.clear();
        self.markdown.borrow_mut().clear();
        self.commands_dismissed = false;
        if let Some(session) = self.sessions.iter().find(|session| session.id == id) {
            self.selected_agent = session.agent.to_string();
            self.selected_directory = session.cwd.as_deref().map(groups::normalize);
            self.collapsed_workspaces.remove(&self.selected_directory);
        }
        self.conversation = None;
        self.approvals.clear();
        self.approval_error = None;
        self.epoch += 1;
        self.stream = StreamState::Loading;
        self.error = None;
        self.editing_title = false;
        self.editing_workspace = false;
        self.expanded.clear();
        self.list.reset(0);
        self.list.set_follow_mode(FollowMode::Tail);
        if restore_draft {
            self.composer_expanded = false;
            self.sync_composer_mode(cx);
            self.composer_generation += 1;
            let draft = self.drafts.get(&id).cloned().unwrap_or_default();
            self.attachments = draft.attachments;
            self.composer.update(cx, |input, cx| {
                input.set_value(draft.text, window, cx);
            });
            self.focus_input(window, cx);
        }
        self.command(Command::Watch {
            session: id,
            epoch: self.epoch,
        });
        cx.notify();
    }

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_thread(self.selected_directory.clone(), window, cx);
    }

    fn new_thread(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.detach_thread(directory, window, cx);
        let draft = self
            .new_drafts
            .get(&self.selected_directory)
            .cloned()
            .unwrap_or_default();
        self.composer_expanded = !self.compact;
        self.sync_composer_mode(cx);
        self.attachments = draft.attachments;
        self.composer.update(cx, |input, cx| {
            input.set_value(draft.text, window, cx);
        });
        self.focus_input(window, cx);
        cx.notify();
    }

    fn detach_thread(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.archived_list = false;
        self.save_draft(cx);
        self.close_reply_preview(window, cx);
        self.active = None;
        self.waiting_for_agent_selection = true;
        self.composer_generation += 1;
        self.restore_on_connect = false;
        self.remember_thread();
        self.conversation = None;
        self.approvals.clear();
        self.approval_error = None;
        self.creating = false;
        self.authentication_required = None;
        self.authentication_panel = None;
        self.authentication_load = None;
        self.elicitation_panels.clear();
        self.markdown.borrow_mut().clear();
        self.commands_dismissed = false;
        self.expanded.clear();
        self.list.reset(0);
        self.epoch += 1;
        self.editing_title = false;
        self.editing_workspace = false;
        self.error = None;
        self.selected_directory = directory;
        if let Some(directory) = &self.selected_directory
            && !self.directories.contains(directory)
        {
            self.directories.push(directory.clone());
        }
        self.collapsed_workspaces.remove(&self.selected_directory);
        self.command(Command::Detach);
    }

    fn current(&self) -> Option<&SessionMeta> {
        self.conversation
            .as_ref()
            .map(|state| &state.session)
            .or_else(|| {
                self.sessions
                    .iter()
                    .find(|session| Some(session.id) == self.active)
            })
    }

    fn agent_icon(
        &self,
        element_id: impl Into<SharedString>,
        agent: &str,
        cx: &App,
    ) -> crate::agent_icon::AgentIcon {
        let uri = self
            .agent_metadata
            .as_ref()
            .and_then(|metadata| metadata.read(cx).icon(self.server, agent))
            .unwrap_or_default();
        crate::agent_icon::AgentIcon::new(element_id, uri).size(px(AGENT_ICON_SIZE))
    }

    fn working(&self) -> bool {
        self.current()
            .is_some_and(|session| session.status == SessionStatus::Working)
    }

    fn can_send(&self, cx: &App) -> bool {
        if self.paste_pending {
            return false;
        }
        if self.approval_error.is_some() {
            return false;
        }
        let ready = self.active.is_some()
            && self.stream == StreamState::Live
            && self.current().is_some_and(|session| {
                matches!(session.status, SessionStatus::Idle | SessionStatus::Working)
                    && !self.archive_blocked(session.id)
                    && !self.cancelling.contains(&session.id)
                    && !self.sending(session.id)
                    && !self.configuring.contains(&session.id)
                    && !self.loading_attachments.contains(&session.id)
            });
        self.connection_status() == status::ConnectionStatus::Connected
            && !self.creating
            && ready
            && self.attachment_error(cx).is_none()
            && !self.composer.read(cx).value().trim().is_empty()
    }

    fn prepare_session(&mut self, cx: &mut Context<Self>) {
        if !self.connected
            || self.archived_list
            || self.active.is_some()
            || self.creating
            || self.waiting_for_agent_selection
            || self.authentication_required.is_some()
            || self.selected_agent.is_empty()
        {
            return;
        }
        let cwd = match self
            .selected_directory
            .as_deref()
            .map(basr::service::utf8_cwd)
            .transpose()
        {
            Err(_) => {
                self.error = Some("工作目录不是有效 UTF-8，无法创建会话".into());
                cx.notify();
                return;
            }
            Ok(path) => path.unwrap_or_default().to_owned(),
        };
        self.creating = true;
        self.command(Command::Create(connection::CreateSession {
            epoch: self.epoch,
            agent: self.selected_agent.clone(),
            title: String::new(),
            cwd,
        }));
        cx.notify();
    }

    fn can_select_agent(&self) -> bool {
        if self.creating || self.paste_pending || self.archived_list {
            return false;
        }
        let Some(id) = self.active else {
            return self.current().is_none();
        };
        !self.thread_has_user_prompt()
            && self.connection_status() == status::ConnectionStatus::Connected
            && self.current().is_some_and(|session| {
                session.status == SessionStatus::Idle && session.active_turn.is_none()
            })
            && self.conversation.as_ref().is_some_and(|conversation| {
                conversation.cursor >= conversation.session.revision.raw()
            })
            && !self.archive_blocked(id)
            && !self.delete_pending.contains(&id)
            && !self.configuring.contains(&id)
            && !self.cancelling.contains(&id)
            && !self.loading_attachments.contains(&id)
            && !self
                .outboxes
                .get(&id)
                .is_some_and(outbox::SessionOutbox::pending)
            && self.approvals.is_empty()
            && self.elicitation_panels.is_empty()
    }

    pub(super) fn thread_has_user_prompt(&self) -> bool {
        // Unknown history is treated as started until a complete replay proves otherwise.
        self.active
            .is_some_and(|id| !self.unstarted_sessions.contains(&id))
    }

    fn record_user_prompt(&mut self, session: SessionId) {
        self.prompted_sessions.insert(session);
        self.unstarted_sessions.remove(&session);
    }

    fn observe_user_prompt(&mut self, event: &basr::model::Event) {
        if matches!(
            event.kind,
            basr::model::EventKind::TurnStarted { .. }
                | basr::model::EventKind::TurnFinished { .. }
                | basr::model::EventKind::StatusChanged {
                    status: SessionStatus::Working
                }
                | basr::model::EventKind::Acp(basr::acp::SessionUpdate::UserMessageChunk(_))
        ) {
            self.record_user_prompt(event.session);
        }
    }

    fn start_agent_chat(&mut self, agent: String, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_agent = agent;
        self.new_thread(self.selected_directory.clone(), window, cx);
        self.waiting_for_agent_selection = false;
        self.prepare_session(cx);
    }

    fn select_agent(&mut self, agent: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_select_agent() {
            return;
        }
        if self.active.is_some() && self.selected_agent == agent {
            self.focus_input(window, cx);
            return;
        }
        self.detach_thread(self.selected_directory.clone(), window, cx);
        self.selected_agent = agent;
        self.waiting_for_agent_selection = false;
        self.prepare_session(cx);
        self.focus_input(window, cx);
        cx.notify();
    }

    fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        let Some(session) = self.active else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        if self.working()
            || self
                .outboxes
                .get(&session)
                .is_some_and(outbox::SessionOutbox::pending)
        {
            self.enqueue_draft(session, text, window, cx);
            return;
        }
        self.error = None;
        if self.composer_fills_thread() {
            self.composer_expanded = false;
            self.sync_composer_mode(cx);
        }
        self.commands_dismissed = true;
        self.list.set_follow_mode(FollowMode::Tail);
        let outbox::Action::Send(draft) = self.outboxes.entry(session).or_default().start(
            Draft {
                text: text.clone(),
                attachments: self.attachments.clone(),
            },
            outbox::Origin::Composer,
        ) else {
            unreachable!("starting a submission returns its network action");
        };
        let title = self.current().filter(|meta| meta.title.is_none()).map(|_| {
            text.lines()
                .find(|line| !line.trim().is_empty())
                .or_else(|| self.attachments.first().map(|item| item.name.as_str()))
                .unwrap_or_default()
                .chars()
                .take(48)
                .collect()
        });
        self.command(Command::Prompt {
            session,
            text: draft.text,
            attachments: draft.attachments,
            capabilities: self.prompt_capabilities(),
            title,
        });
        cx.notify();
    }

    fn update(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        match update {
            Update::Elicitations {
                epoch,
                pending,
                completed_ids,
            } if epoch == self.epoch && self.active.is_some() => {
                self.sync_elicitations(pending, completed_ids, window, cx);
            }
            Update::ElicitationUnavailable { epoch, message } if epoch == self.epoch => {
                if !self.elicitation_panels.is_empty() {
                    self.error = Some(message);
                }
                self.elicitation_panels.clear();
            }
            Update::ElicitationResponded {
                session,
                request_id,
                result,
            } => {
                if self.active == Some(session)
                    && let Err(error) = result
                    && let Some(panel) = self.elicitation_panels.get(&request_id)
                {
                    panel.update(cx, |panel, cx| panel.failed(error, cx));
                }
            }
            Update::Configured { session, result } => {
                self.configuring.remove(&session);
                match result {
                    Ok(_) => { /* Configuration state arrives in the ordered event stream. */ }
                    Err(error) if self.active == Some(session) => self.error = Some(error),
                    Err(_) => {}
                }
            }
            Update::Catalog(result) => match result {
                Ok(mut catalog) => {
                    if !self.connected {
                        crate::agent_metadata::ensure(self.server, cx);
                    }
                    self.connected = true;
                    self.connection_error = None;
                    // Session IDs are never reused. Older snapshots cannot revive
                    // a session whose removal has already been confirmed.
                    catalog
                        .sessions
                        .retain(|meta| !self.removed_sessions.contains(&meta.id));
                    let catalog_ids: HashSet<_> =
                        catalog.sessions.iter().map(|meta| meta.id).collect();
                    // Catalog requests are serial, but create receipts can overtake them.
                    // Only disappearance after a prior catalog observation proves removal.
                    for removed in self.catalog_ids.difference(&catalog_ids) {
                        self.drafts.remove(removed);
                        self.outboxes.remove(removed);
                        self.unstarted_sessions.remove(removed);
                        self.prompted_sessions.remove(removed);
                        self.removed_sessions.insert(*removed);
                    }
                    self.catalog_ids = catalog_ids;
                    for meta in &mut catalog.sessions {
                        if let Some(old) = self.sessions.iter().find(|old| old.id == meta.id) {
                            let incoming = std::mem::replace(meta, old.clone());
                            crate::conversation::merge_meta(meta, &incoming);
                        }
                        if let Some(conversation) = &self.conversation
                            && conversation.session.id == meta.id
                        {
                            crate::conversation::merge_meta(meta, &conversation.session);
                        }
                        if meta.status == SessionStatus::Working || meta.active_turn.is_some() {
                            self.record_user_prompt(meta.id);
                        }
                    }
                    catalog
                        .sessions
                        .sort_by_key(|session| std::cmp::Reverse(session.updated_at));
                    self.sessions = catalog.sessions;
                    self.agents = catalog.agents;
                    // An older catalog can arrive after choosing a newly installed agent.
                    if !self.creating
                        && self.authentication_required.is_none()
                        && !self
                            .agents
                            .iter()
                            .any(|agent| agent.id == self.selected_agent)
                    {
                        self.selected_agent = self
                            .agents
                            .first()
                            .map(|agent| agent.id.clone())
                            .unwrap_or_default();
                        self.authentication_required = None;
                    }
                    if let Some(conversation) = &mut self.conversation
                        && let Some(meta) = self
                            .sessions
                            .iter()
                            .find(|session| session.id == conversation.session.id)
                    {
                        conversation.merge_meta(meta);
                    }
                    self.restore_thread(window, cx);
                    if self.error.is_none() {
                        self.prepare_session(cx);
                    }
                }
                Err(error) => {
                    self.connected = false;
                    self.connection_error = Some(error);
                }
            },
            Update::Created { epoch, result } => {
                let current = epoch == self.epoch && self.active.is_none();
                if current {
                    self.creating = false;
                }
                match result {
                    Ok(session) if self.removed_sessions.contains(&session.id) => {
                        if current {
                            self.error =
                                Some("创建的 thread 已被删除，请新建 thread 后继续。".into());
                        }
                    }
                    Ok(session) => {
                        let id = session.id;
                        if current
                            && session.status != SessionStatus::Working
                            && session.active_turn.is_none()
                            && !self.prompted_sessions.contains(&id)
                        {
                            self.unstarted_sessions.insert(id);
                        }
                        if session.status == SessionStatus::Working || session.active_turn.is_some()
                        {
                            self.record_user_prompt(id);
                        }
                        if let Some(meta) = self.sessions.iter_mut().find(|meta| meta.id == id) {
                            crate::conversation::merge_meta(meta, &session);
                        } else {
                            self.sessions.insert(0, session);
                        }
                        if current {
                            let draft = self.composer.read(cx).value().to_string();
                            self.drafts.insert(
                                id,
                                Draft {
                                    text: draft,
                                    attachments: self.attachments.clone(),
                                },
                            );
                            let directory = self.selected_directory.clone();
                            self.activate(id, false, window, cx);
                            self.new_drafts.remove(&directory);
                        }
                    }
                    Err(error) if current => {
                        self.error =
                            Some(format!("初始化 Agent 失败，草稿已保留；F5 重试：{error}"));
                    }
                    Err(_) => {}
                }
            }
            Update::AuthenticationRequired { epoch, agent } => {
                if epoch == self.epoch && self.active.is_none() && self.selected_agent == agent {
                    self.creating = false;
                    self.error = None;
                    self.authentication_required = Some(agent);
                    self.load_authentication_panel(window, cx);
                }
            }
            Update::Renamed { session, result } => {
                self.renaming = false;
                match result {
                    Ok(()) => {
                        if self.active == Some(session) && self.editing_title {
                            self.editing_title = false;
                            self.focus_input(window, cx);
                        }
                        // The ordered event or catalog carries the title's revision.
                    }
                    Err(error) if self.active == Some(session) && self.editing_title => {
                        self.error = Some(format!("重命名失败：{error}"));
                        self.rename.update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(_) => {}
                }
            }
            Update::Archived { session, result } => {
                self.archive_pending.remove(&session);
                match result {
                    Ok(incoming) => {
                        // A newer restore snapshot must not erase this successful archive's
                        // queue pause, even when its metadata revision wins the merge.
                        if incoming.archived
                            && let Some(outbox) = self.outboxes.get_mut(&session)
                        {
                            outbox.pause();
                        }
                        // A late receipt must not resurrect a deleted catalog entry.
                        if let Some(meta) = self.sessions.iter_mut().find(|meta| meta.id == session)
                        {
                            crate::conversation::merge_meta(meta, &incoming);
                            if let Some(conversation) = &mut self.conversation
                                && conversation.session.id == session
                            {
                                conversation.merge_meta(meta);
                            }
                        }
                    }
                    Err(error) => {
                        self.operation_error(
                            "归档状态更新失败",
                            format!("草稿和队列已保留。\n{error}"),
                            cx,
                        );
                    }
                }
            }
            Update::Deleted { session, result } => {
                self.delete_pending.remove(&session);
                match result {
                    Ok(()) => {
                        self.removed_sessions.insert(session);
                        self.sessions.retain(|meta| meta.id != session);
                        self.catalog_ids.remove(&session);
                        if self.active == Some(session) {
                            if let Some(next) = self
                                .sessions
                                .iter()
                                .find(|meta| !meta.archived)
                                .map(|meta| meta.id)
                            {
                                self.select(next, window, cx);
                            } else {
                                self.start_agent_chat(self.selected_agent.clone(), window, cx);
                            }
                        }
                        self.drafts.remove(&session);
                        self.outboxes.remove(&session);
                        self.unstarted_sessions.remove(&session);
                        self.prompted_sessions.remove(&session);
                        self.seen_replies.remove(&session);
                        if self.pinned.contains(&session) {
                            self.toggle_pinned(session, cx);
                        }
                    }
                    Err(error) => {
                        self.operation_error(
                            "删除失败",
                            format!("会话 {session} 的本地草稿和队列已保留。\n{error}"),
                            cx,
                        );
                    }
                }
            }
            Update::Sent {
                session,
                text,
                attachments,
                result,
            } => {
                if result.is_ok() {
                    self.record_user_prompt(session);
                }
                let action = self
                    .outboxes
                    .get_mut(&session)
                    .and_then(|outbox| outbox.receipt(result.as_ref().ok().copied()));
                match action {
                    Some(outbox::Action::Accepted(outbox::Origin::Composer)) => {
                        if let Some(draft) = self.drafts.get_mut(&session) {
                            if draft.text == text {
                                draft.text.clear();
                            }
                            draft
                                .attachments
                                .retain(|item| !attachments.iter().any(|sent| sent.id == item.id));
                        }
                        if self.active == Some(session) {
                            self.attachments
                                .retain(|item| !attachments.iter().any(|sent| sent.id == item.id));
                        }
                        if self.active == Some(session)
                            && self.composer.read(cx).value().as_ref() == text
                        {
                            self.composer
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                    }
                    Some(outbox::Action::Rejected { origin, draft }) => {
                        if origin == outbox::Origin::Composer {
                            self.drafts.insert(session, draft);
                        }
                        let error = result.expect_err("rejection requires an RPC error");
                        self.error =
                            Some(format!("发送失败，草稿已保留（会话 {session}）：{error}"))
                    }
                    _ => {}
                }
            }
            Update::Cancelled { session, result } => {
                self.cancelling.remove(&session);
                self.complete_queue_interrupt(session, result.is_ok());
                if let Err(error) = result {
                    self.error = Some(format!("停止失败：{error}"));
                }
            }
            Update::Snapshot {
                epoch,
                session,
                events,
            } if epoch == self.epoch && self.active == Some(session.id) => {
                self.markdown.borrow_mut().clear();
                let known = self
                    .sessions
                    .iter()
                    .find(|meta| meta.id == session.id)
                    .unwrap_or(&session);
                let mut conversation = Conversation::new(known.clone());
                conversation.merge_meta(&session);
                if let Some(old) = &self.conversation
                    && old.session.id == session.id
                {
                    conversation.merge_meta(&old.session);
                }
                let incomplete_history = events.iter().enumerate().any(|(index, event)| {
                    event.session != session.id || event.cursor.raw() != index as u64 + 1
                });
                if incomplete_history {
                    self.unstarted_sessions.remove(&session.id);
                } else if !events.is_empty()
                    && events
                        .last()
                        .is_some_and(|event| event.cursor >= conversation.session.revision)
                    && !self.prompted_sessions.contains(&session.id)
                {
                    self.unstarted_sessions.insert(session.id);
                }
                if conversation.session.status == SessionStatus::Working
                    || conversation.session.active_turn.is_some()
                {
                    self.record_user_prompt(session.id);
                }
                for event in events {
                    self.observe_user_prompt(&event);
                    if let Some(outbox) = self.outboxes.get_mut(&event.session) {
                        outbox.observe(&event);
                    }
                    conversation.apply(event);
                }
                if let Some(meta) = self.sessions.iter_mut().find(|meta| meta.id == session.id) {
                    crate::conversation::merge_meta(meta, &conversation.session);
                }
                self.list.reset(conversation.rows.len());
                self.list.set_follow_mode(FollowMode::Tail);
                self.conversation = Some(conversation);
            }
            Update::Event { epoch, event } if epoch == self.epoch => {
                self.observe_user_prompt(&event);
                if let Some(meta) = self
                    .sessions
                    .iter_mut()
                    .find(|meta| meta.id == event.session)
                {
                    meta.apply_event(&event);
                }
                if let Some(outbox) = self.outboxes.get_mut(&event.session) {
                    outbox.observe(&event);
                }
                if let Some(conversation) = &mut self.conversation {
                    let old_count = conversation.rows.len();
                    let expanded_activity = conversation
                        .rows
                        .last()
                        .filter(|row| row.activity && row.items.len() == 1)
                        .map(|row| conversation.items[row.items.start].id())
                        .filter(|id| self.expanded.contains(id));
                    let changed = conversation.apply(event);
                    let count = conversation.rows.len();
                    if let Some(id) = expanded_activity
                        && conversation
                            .rows
                            .get(old_count.saturating_sub(1))
                            .is_some_and(|row| row.items.len() > 1)
                    {
                        self.expanded.insert(format!("activity-{id}"));
                    }
                    if count > old_count {
                        self.list.splice(old_count..old_count, count - old_count);
                    }
                    if let Some(index) = changed.and_then(|index| conversation.row_for_item(index))
                    {
                        self.list.remeasure_items(index..count);
                    }
                    if let Some(meta) = self
                        .sessions
                        .iter_mut()
                        .find(|meta| meta.id == conversation.session.id)
                    {
                        crate::conversation::merge_meta(meta, &conversation.session);
                    }
                }
            }
            Update::Stream { epoch, state } if epoch == self.epoch => self.stream = state,
            Update::Permissions { epoch, pending } if epoch == self.epoch => {
                self.approval_error = None;
                self.approvals.replace(pending);
                if self.compact && !self.approvals.is_empty() {
                    self.output_open = true;
                    self.resize_window(window, cx);
                }
            }
            Update::PermissionResponded {
                session,
                request_id,
                result,
            } => {
                if self.active == Some(session) {
                    self.approvals.responded(&request_id, result);
                }
            }
            Update::PermissionUnavailable { epoch, message } if epoch == self.epoch => {
                self.approvals.clear();
                self.approval_error = Some(message);
                if let Some(session) = self.active
                    && let Some(outbox) = self.outboxes.get_mut(&session)
                {
                    outbox.pause();
                }
            }
            Update::Snapshot { .. }
            | Update::Event { .. }
            | Update::Stream { .. }
            | Update::Permissions { .. }
            | Update::PermissionUnavailable { .. } => {}
            Update::Elicitations { .. } | Update::ElicitationUnavailable { .. } => {}
        }
        self.pause_archived();
        self.flush_queue(cx);
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.measure_composer(cx);
        self.sync_foreground(window);
        self.sync_reply_visibility(window, cx);
        if self.compact {
            self.resize_window(window, cx);
        }
        self.render_layout(window, cx)
    }
}

fn title(session: &SessionMeta) -> String {
    session
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "新 thread".into())
}

fn matches_query(session: &SessionMeta, query: &str) -> bool {
    query.is_empty()
        || title(session).to_lowercase().contains(query)
        || session.agent.as_str().to_lowercase().contains(query)
        || session.id.to_string().contains(query)
        || session
            .cwd
            .as_ref()
            .is_some_and(|path| path.to_string_lossy().to_lowercase().contains(query))
}

fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
