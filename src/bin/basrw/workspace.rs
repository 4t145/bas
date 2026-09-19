mod attachments;
mod chat;
mod commands;
mod forms;
mod groups;
mod layout;
mod markdown;
mod menus;
#[cfg(feature = "gui-tests")]
mod preview;
mod queue;
mod reply;
mod restore;
mod sidebar;
mod status;
mod thought;

use std::{
    cell::RefCell,
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
    component::{button::*, input::*, text::TextView, *},
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
        Dismiss,
        Refresh
    ]
);

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-n", NewSession, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-f", FocusSearch, Some("BasrWorkspace")),
        // Override Input's own search action while editing a BASR field.
        KeyBinding::new("ctrl-f", FocusSearch, Some("BasrWorkspace > Input")),
        KeyBinding::new("ctrl-l", FocusComposer, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-b", ToggleSidebar, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-shift-m", ToggleMode, Some("BasrWorkspace")),
        KeyBinding::new("ctrl-o", ToggleOutput, Some("BasrWorkspace")),
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
    foreground: Option<SessionId>,
    connection: Connection,
    restore_store: Option<restore::Store>,
    restore_on_connect: bool,
    server: SocketAddr,
    sessions: Vec<SessionMeta>,
    agents: Vec<pb::Agent>,
    active: Option<SessionId>,
    conversation: Option<Conversation>,
    epoch: u64,
    stream: StreamState,
    connected: bool,
    connection_error: Option<String>,
    error: Option<String>,
    search: Entity<InputState>,
    composer: Entity<EditorState>,
    mention_decorations: TextDecorationCollection,
    decorated_mentions: Vec<TextDecoration>,
    mention_selection: std::ops::Range<usize>,
    cwd: Entity<InputState>,
    rename: Entity<InputState>,
    selected_agent: String,
    drafts: HashMap<SessionId, Draft>,
    attachments: Vec<Attachment>,
    loading_attachments: HashSet<SessionId>,
    new_drafts: HashMap<Option<PathBuf>, Draft>,
    directories: Vec<PathBuf>,
    selected_directory: Option<PathBuf>,
    collapsed_workspaces: HashSet<Option<PathBuf>>,
    command_selection: usize,
    commands_dismissed: bool,
    markdown: RefCell<HashMap<String, markdown::Document>>,
    sending: HashSet<SessionId>,
    queued: HashMap<SessionId, std::collections::VecDeque<Draft>>,
    queued_inflight: HashMap<SessionId, Draft>,
    queued_paused: HashSet<SessionId>,
    awaiting_turn: HashMap<SessionId, basr::model::Timestamp>,
    configuring: HashSet<SessionId>,
    cancelling: HashSet<SessionId>,
    creating: bool,
    editing_title: bool,
    renaming: bool,
    deleting: Option<SessionId>,
    delete_pending: bool,
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
    _theme_watch: Option<Task<()>>,
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
        view.restore_on_connect = true;
        view.placement = Some(desktop::Placement::new(runtime));
        view._theme_watch = Some(theme::watch(window, cx));
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
                this.remember_thread();
                this.close_reply_preview(window, cx);
                this.close_menu(window, cx);
            });
            true
        });
        let selected_directory = Some(groups::normalize(&cwd));
        let directories = vec![groups::normalize(&cwd)];
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索 threads…"));
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
                .submit_on_enter(true)
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
        let subscriptions = vec![
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
                        .sessions
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
                        .map_or(0, |state| state.items.len()),
                );
                cx.notify();
            }),
            cx.observe(&composer, |this, _, cx| {
                this.sync_mentions(cx);
                cx.notify();
            }),
            cx.observe(&rename, |_, _, cx| cx.notify()),
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. })
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
            agents: Vec::new(),
            active: None,
            conversation: None,
            epoch: 0,
            stream: StreamState::Loading,
            connected: false,
            connection_error: None,
            error: None,
            search,
            composer,
            mention_decorations,
            decorated_mentions: Vec::new(),
            mention_selection: 0..0,
            cwd,
            rename,
            selected_agent: String::new(),
            drafts: HashMap::new(),
            attachments: Vec::new(),
            loading_attachments: HashSet::new(),
            new_drafts: HashMap::new(),
            selected_directory,
            directories,
            collapsed_workspaces: HashSet::new(),
            command_selection: 0,
            commands_dismissed: false,
            markdown: RefCell::new(HashMap::new()),
            sending: HashSet::new(),
            queued: HashMap::new(),
            queued_inflight: HashMap::new(),
            queued_paused: HashSet::new(),
            awaiting_turn: HashMap::new(),
            configuring: HashSet::new(),
            cancelling: HashSet::new(),
            creating: false,
            editing_title: false,
            renaming: false,
            deleting: None,
            delete_pending: false,
            window: window.window_handle(),
            menu: None,
            reply_preview: None,
            reply_hover: None,
            seen_replies: HashMap::new(),
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
            _theme_watch: None,
        }
    }

    fn command(&mut self, command: Command) {
        self.error = None;
        if !self.connection.send(command) {
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
        let hint = if self.connection_status() != status::ConnectionStatus::Connected {
            "连接未就绪，暂不能发送"
        } else if self.creating {
            "正在创建 thread，暂不能发送"
        } else if self.active.is_some_and(|id| self.sending.contains(&id)) {
            "正在发送…"
        } else if self.working() {
            "加入待发送队列 · Enter\n当前回复结束后发送 · Shift Enter 换行"
        } else {
            "发送 · Enter\n换行 · Shift Enter"
        };
        self.composer_error(cx)
            .map(|error| format!("{error}\n\n{hint}"))
            .unwrap_or_else(|| hint.into())
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.command(Command::Refresh);
        if self.active.is_none() {
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
        self.restore_on_connect = false;
        self.remember_thread();
        self.creating = false;
        self.markdown.borrow_mut().clear();
        self.commands_dismissed = false;
        if let Some(session) = self.sessions.iter().find(|session| session.id == id) {
            self.selected_agent = session.agent.to_string();
            self.selected_directory = session.cwd.as_deref().map(groups::normalize);
            self.collapsed_workspaces.remove(&self.selected_directory);
        }
        self.conversation = None;
        self.epoch += 1;
        self.stream = StreamState::Loading;
        self.error = None;
        self.editing_title = false;
        self.editing_workspace = false;
        self.deleting = None;
        self.expanded.clear();
        self.list.reset(0);
        self.list.set_follow_mode(FollowMode::Tail);
        if restore_draft {
            let draft = self.drafts.get(&id).cloned().unwrap_or_default();
            self.attachments = draft.attachments;
            self.composer.update(cx, |input, cx| {
                input.set_value(draft.text, window, cx);
                input.focus(window, cx);
            });
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
        self.save_draft(cx);
        self.close_reply_preview(window, cx);
        self.active = None;
        self.restore_on_connect = false;
        self.remember_thread();
        self.conversation = None;
        self.creating = false;
        self.markdown.borrow_mut().clear();
        self.commands_dismissed = false;
        self.expanded.clear();
        self.list.reset(0);
        self.epoch += 1;
        self.deleting = None;
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
        let draft = self
            .new_drafts
            .get(&self.selected_directory)
            .cloned()
            .unwrap_or_default();
        self.command(Command::Detach);
        self.attachments = draft.attachments;
        self.composer.update(cx, |input, cx| {
            input.set_value(draft.text, window, cx);
            input.focus(window, cx);
        });
        self.prepare_session(cx);
        cx.notify();
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

    fn working(&self) -> bool {
        self.current()
            .is_some_and(|session| session.status == SessionStatus::Working)
    }

    fn can_send(&self, cx: &App) -> bool {
        let ready = self.active.is_some()
            && self.stream == StreamState::Live
            && self.current().is_some_and(|session| {
                matches!(session.status, SessionStatus::Idle | SessionStatus::Working)
                    && !self.cancelling.contains(&session.id)
                    && !self.sending.contains(&session.id)
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
            || self.active.is_some()
            || self.creating
            || self.selected_agent.is_empty()
        {
            return;
        }
        self.creating = true;
        self.command(Command::Create(connection::CreateSession {
            epoch: self.epoch,
            agent: self.selected_agent.clone(),
            title: String::new(),
            cwd: self
                .selected_directory
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }));
        cx.notify();
    }

    fn select_agent(&mut self, agent: String, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .current()
            .is_some_and(|session| session.agent.as_str() == agent)
            || (self.active.is_none() && self.selected_agent == agent && self.creating)
        {
            return;
        }
        let draft = self.composer.read(cx).value().to_string();
        let attachments = self.attachments.clone();
        self.selected_agent = agent;
        self.new_thread(self.selected_directory.clone(), window, cx);
        self.attachments = attachments;
        self.composer
            .update(cx, |input, cx| input.set_value(draft, window, cx));
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
            || self.awaiting_turn.contains_key(&session)
            || self
                .queued
                .get(&session)
                .is_some_and(|queue| !queue.is_empty())
        {
            self.enqueue_draft(session, text, window, cx);
            return;
        }
        if let Some(meta) = self.current() {
            self.awaiting_turn.insert(session, meta.updated_at);
        }
        self.error = None;
        self.commands_dismissed = true;
        self.list.set_follow_mode(FollowMode::Tail);
        self.sending.insert(session);
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
            text,
            attachments: self.attachments.clone(),
            capabilities: self.prompt_capabilities(),
            title,
        });
        cx.notify();
    }

    fn update(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        match update {
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
                    self.connected = true;
                    self.connection_error = None;
                    catalog
                        .sessions
                        .sort_by_key(|session| std::cmp::Reverse(session.updated_at));
                    self.sessions = catalog.sessions;
                    self.agents = catalog.agents;
                    if !self
                        .agents
                        .iter()
                        .any(|agent| agent.id == self.selected_agent)
                    {
                        self.selected_agent = self
                            .agents
                            .first()
                            .map(|agent| agent.id.clone())
                            .unwrap_or_default();
                    }
                    if let Some(conversation) = &mut self.conversation
                        && let Some(meta) = self
                            .sessions
                            .iter()
                            .find(|session| session.id == conversation.session.id)
                        && meta.updated_at > conversation.session.updated_at
                    {
                        conversation.session = meta.clone();
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
                    Ok(session) => {
                        let id = session.id;
                        self.sessions.retain(|meta| meta.id != id);
                        self.sessions.insert(0, session);
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
            Update::Renamed {
                session,
                title,
                result,
            } => {
                self.renaming = false;
                match result {
                    Ok(()) => {
                        if self.active == Some(session) && self.editing_title {
                            self.editing_title = false;
                            self.focus_input(window, cx);
                        }
                        if let Some(meta) = self.sessions.iter_mut().find(|meta| meta.id == session)
                        {
                            meta.title = Some(title.clone());
                        }
                        if let Some(conversation) = self
                            .conversation
                            .as_mut()
                            .filter(|c| c.session.id == session)
                        {
                            conversation.session.title = Some(title);
                        }
                    }
                    Err(error) => self.error = Some(format!("重命名失败：{error}")),
                }
            }
            Update::Deleted { session, result } => {
                self.delete_pending = false;
                match result {
                    Ok(()) => {
                        self.sessions.retain(|meta| meta.id != session);
                        if self.active == Some(session) {
                            self.new_session(window, cx);
                        }
                        self.drafts.remove(&session);
                        self.sending.remove(&session);
                        self.queued.remove(&session);
                        self.queued_inflight.remove(&session);
                        self.queued_paused.remove(&session);
                        self.awaiting_turn.remove(&session);
                        self.deleting = None;
                    }
                    Err(error) => self.error = Some(format!("删除会话失败：{error}")),
                }
            }
            Update::Sent {
                session,
                text,
                attachments,
                result,
            } => {
                self.sending.remove(&session);
                if let Some(draft) = self.queued_inflight.remove(&session) {
                    if let Err(error) = result {
                        self.awaiting_turn.remove(&session);
                        self.queued_paused.insert(session);
                        self.queued.entry(session).or_default().push_front(draft);
                        self.error = Some(format!("追加消息发送失败，消息已保留：{error}"));
                    }
                    self.flush_queue(cx);
                    cx.notify();
                    return;
                }
                if result.is_err() {
                    self.awaiting_turn.remove(&session);
                }
                match result {
                    Ok(()) => {
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
                    Err(error) => {
                        self.error =
                            Some(format!("发送失败，草稿已保留（会话 {session}）：{error}"))
                    }
                }
            }
            Update::Cancelled { session, result } => {
                self.cancelling.remove(&session);
                if let Err(error) = result {
                    self.error = Some(format!("停止失败：{error}"));
                }
            }
            Update::Snapshot {
                epoch,
                session,
                events,
            } if epoch == self.epoch => {
                self.markdown.borrow_mut().clear();
                let mut conversation = Conversation::new(session.clone());
                for event in events {
                    conversation.apply(event);
                }
                if let Some(update) = &session.available_commands {
                    conversation.commands = update.available_commands.clone();
                }
                conversation.session = session;
                self.list.reset(conversation.items.len());
                self.list.set_follow_mode(FollowMode::Tail);
                self.conversation = Some(conversation);
            }
            Update::Event { epoch, event } if epoch == self.epoch => {
                if let Some(conversation) = &mut self.conversation {
                    if event.cursor.raw() > conversation.cursor
                        && matches!(
                            event.kind,
                            basr::model::EventKind::TurnStarted { .. }
                                | basr::model::EventKind::TurnFinished { .. }
                        )
                    {
                        self.awaiting_turn.remove(&event.session);
                    }
                    let old_count = conversation.items.len();
                    let changed = conversation.apply(event);
                    let count = conversation.items.len();
                    if count > old_count {
                        self.list.splice(old_count..old_count, count - old_count);
                    }
                    if let Some(index) = changed {
                        self.list.remeasure_items(index..count);
                    }
                    if let Some(meta) = self
                        .sessions
                        .iter_mut()
                        .find(|meta| meta.id == conversation.session.id)
                    {
                        *meta = conversation.session.clone();
                    }
                }
            }
            Update::Stream { epoch, state } if epoch == self.epoch => self.stream = state,
            Update::Snapshot { .. } | Update::Event { .. } | Update::Stream { .. } => {}
        }
        self.flush_queue(cx);
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

fn status(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Starting => "启动中",
        SessionStatus::Idle => "就绪",
        SessionStatus::Working => "运行中",
        SessionStatus::Stopped => "已停止",
        SessionStatus::Failed => "失败",
    }
}

fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
