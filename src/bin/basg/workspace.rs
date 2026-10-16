mod agent;
mod application;
mod archive;
mod attachments;
mod chat;
mod commands;
mod composer;
mod header;
mod history;
mod images;
mod input_layout;
mod interaction;
mod layout;
mod markdown;
mod menus;
mod message_actions;
mod navigation;
mod operations;
mod outbox;
#[cfg(feature = "gui-tests")]
mod preview;
mod protocol_elements;
mod queue;
mod rename;
mod reply;
mod runtime;
mod settings;
mod state;
mod status;
mod submission;
mod thought;
mod thread_actions;
mod tool_content;
mod turn_navigation;
mod updates;
mod usage;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
};

use bas::model::{SessionId, SessionMeta, SessionStatus};
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
    connection::{self, Command, Connection, Failure, StreamState, Update},
    conversation::{Conversation, Item},
    theme,
    ui::Button,
};

mod shortcuts;

const EVENT_BATCH: usize = 64;
const LIST_OVERDRAW: f32 = 800.;
const SIDEBAR_WIDTH: f32 = 252.;
const AGENT_ICON_SIZE: f32 = 16.;
const SIDEBAR_TIME_REFRESH: std::time::Duration = crate::ui::time::REFRESH_INTERVAL;

use crate::application::OpenSettings;
pub(super) use layout::initial_size;

actions!(
    bas,
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
        Refresh,
        ScrollConversationToEnd
    ]
);

pub(super) fn bind_keys(cx: &mut App) {
    composer::bind_keys(cx);
    shortcuts::bind_keys(cx);
    cx.bind_keys([
        KeyBinding::new("ctrl-n", NewSession, Some("BasWorkspace")),
        KeyBinding::new("ctrl-f", FocusSearch, Some("BasWorkspace")),
        // Override Input's own search action while editing a BasG field.
        KeyBinding::new("ctrl-f", FocusSearch, Some("BasWorkspace > Input")),
        KeyBinding::new("ctrl-l", FocusComposer, Some("BasWorkspace")),
        KeyBinding::new("ctrl-b", ToggleSidebar, Some("BasWorkspace")),
        KeyBinding::new("ctrl-shift-m", ToggleMode, Some("BasWorkspace")),
        KeyBinding::new("ctrl-o", ToggleOutput, Some("BasWorkspace")),
        KeyBinding::new("ctrl-tab", NextThread, Some("BasWorkspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousThread, Some("BasWorkspace")),
        KeyBinding::new("ctrl-tab", NextThread, Some("BasWorkspace > Input")),
        KeyBinding::new(
            "ctrl-shift-tab",
            PreviousThread,
            Some("BasWorkspace > Input"),
        ),
        KeyBinding::new("escape", Dismiss, Some("BasWorkspace")),
        KeyBinding::new("f5", Refresh, Some("BasWorkspace")),
        KeyBinding::new("ctrl-end", ScrollConversationToEnd, Some("BasWorkspace")),
        KeyBinding::new(
            "ctrl-end",
            ScrollConversationToEnd,
            Some("BasWorkspace > Input"),
        ),
        KeyBinding::new(
            "ctrl-end",
            ScrollConversationToEnd,
            Some("BasWorkspace > BasComposer > Input"),
        ),
    ]);
}

pub(super) struct Workspace {
    window: state::WindowState,
    backend: state::BackendState,
    navigation: state::NavigationState,
    sessions: state::SessionStore,
    selected: state::SelectedThread,
    composer: state::ComposerState,
    renaming: state::RenameState,
    _subscriptions: Vec<Subscription>,
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
        view.attach_restore(navigation::restore::Store::discover(server));
        view.resize_window(window, cx);
        view.navigation._clock = Some(cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(SIDEBAR_TIME_REFRESH).await;
                if view
                    .update_in(cx, |this, _, cx| {
                        if this.window.sidebar_visible && !this.window.compact {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
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
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder(crate::i18n::tr!("session-search")));
        let composer_state = state::ComposerState::new(window, cx, compact);
        let composer = composer_state.input.clone();
        let renaming = state::RenameState::new(window, cx);
        if cx.try_global::<crate::agents::Changes>().is_none() {
            cx.set_global(crate::agents::Changes::default());
        }
        crate::agents::metadata::ensure(server, cx);
        let agent_metadata = crate::agents::metadata::entity(cx);
        let mut subscriptions = vec![
            cx.on_app_quit(|this, _| {
                this.persist_read_state();
                std::future::ready(())
            }),
            cx.observe_global_in::<crate::agents::Changes>(window, |this, window, cx| {
                let changes = cx.global::<crate::agents::Changes>().clone();
                if changes.server != Some(this.backend.server) {
                    return;
                }
                this.command(Command::Refresh, cx);
                if changes.target == Some(this.window.handle.window_id())
                    && let Some(agent) = changes.select
                {
                    this.start_agent_chat(agent, window, cx);
                    window.activate_window();
                    return;
                }
                if changes.ready.as_ref().is_some_and(|agent| {
                    this.selected.authentication.required.as_ref() == Some(agent)
                }) {
                    this.refresh(cx);
                }
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.sync_foreground(window);
                this.resume_workspace_picker(window, cx);
                if !window.is_window_active() {
                    this.close_reply_preview(window, cx);
                }
                cx.notify();
            }),
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.subscribe_in(&search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let query = this.navigation.search.read(cx).value().to_lowercase();
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
                this.selected.timeline.list.remeasure_items(
                    0..this
                        .selected
                        .timeline
                        .conversation
                        .as_ref()
                        .map_or(0, |state| state.rows.len()),
                );
                cx.notify();
            }),
            cx.observe_global_in::<theme::Appearance>(window, |this, window, cx| {
                this.sync_composer_mode(cx);
                for (input, label) in [
                    (&this.navigation.search, crate::i18n::tr!("session-search")),
                    (&this.renaming.input, crate::i18n::tr!("session-title")),
                ] {
                    input.update(cx, |input, cx| input.set_placeholder(label, window, cx));
                }
                this.composer.input.update(cx, |input, cx| {
                    input.set_placeholder(crate::i18n::tr!("composer-placeholder"), window, cx)
                });
                this.close_menu(window, cx);
                window.set_window_title(if this.window.compact {
                    crate::i18n::tr!("app-quick-input")
                } else {
                    "BasG"
                });
                cx.notify();
            }),
            cx.observe(&composer, |this, _, cx| {
                this.sync_mentions(cx);
                cx.notify();
            }),
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                if this.composer.mode == composer::Mode::Send
                    && matches!(event, InputEvent::PressEnter { shift: false, .. })
                    && !this.complete_command(window, cx)
                {
                    this.send_prompt(window, cx);
                }
                if matches!(event, InputEvent::Change) {
                    if this.selected.id.is_none() {
                        this.navigation
                            .drafts
                            .metadata
                            .entry(this.navigation.selected_directory.clone())
                            .or_default()
                            .updated_at = bas::model::Timestamp::now();
                    }
                    this.clear_feedback();
                    this.composer.commands.selection = 0;
                    this.composer.commands.dismissed = false;
                    if this.window.compact {
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
                        if this.window.compact {
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
            window: state::WindowState::new(window.window_handle(), compact),
            backend: state::BackendState::new(connection, server, agent_metadata, event_task),
            navigation: state::NavigationState::new(search, cwd),
            sessions: state::SessionStore::default(),
            selected: state::SelectedThread::new(list),
            composer: composer_state,
            renaming,
            _subscriptions: subscriptions,
        }
    }

    fn sync_foreground(&mut self, window: &Window) {
        let foreground = self.selected.id.filter(|_| window.is_window_active());
        if self.backend.foreground != foreground {
            self.backend.foreground = foreground;
            let _ = self
                .backend
                .connection
                .send(Command::Foreground(foreground));
        }
    }

    fn composer_error(&self, cx: &App) -> Option<String> {
        self.selected
            .id
            .and_then(|id| self.sessions.runtimes.failure(id))
            .map(ToString::to_string)
            .or_else(|| self.selected.error.clone())
            .or_else(|| {
                self.selected
                    .id
                    .and_then(|id| self.sessions.errors.get(&id).cloned())
            })
            .or_else(|| self.selected.approval.error.clone())
            .or_else(|| self.authentication_hint())
            .or_else(|| self.backend.error.as_ref().map(Failure::presentation))
            .or_else(|| match &self.selected.stream {
                StreamState::Reconnecting(error) | StreamState::Unavailable(error) => {
                    Some(error.presentation())
                }
                _ => None,
            })
            .or_else(|| self.attachment_error(cx))
    }

    fn send_hint(&self, _: &App) -> String {
        if self.working() {
            crate::i18n::tr!(
                "composer-queue-hint",
                shortcut = self.composer_send_shortcut()
            )
        } else {
            crate::i18n::tr!(
                "composer-send-hint",
                shortcut = self.composer_send_shortcut()
            )
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.command(Command::Refresh, cx);
        if self.selected.id.is_none() {
            if self.selected.authentication.required.is_some() {
                self.selected.epoch += 1;
                self.selected.creating = false;
                self.selected.authentication.clear();
            }
            self.selected.error = None;
            self.prepare_session(cx);
        }
        if let Some(session) = self.selected.id {
            self.selected.reload();
            self.command(
                Command::Watch {
                    session,
                    epoch: self.selected.epoch,
                },
                cx,
            );
            cx.notify();
        }
    }

    fn save_draft(&mut self, cx: &App) {
        let draft = self.composer.draft(cx);
        if let Some(id) = self.selected.id {
            if !self.sessions.removed.contains(&id) {
                self.sessions.drafts.insert(id, draft);
            }
        } else {
            self.navigation
                .drafts
                .contents
                .insert(self.navigation.selected_directory.clone(), draft);
        }
    }

    fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window.compact {
            self.toggle_mode(window, cx);
            self.window.sidebar_visible = true;
        } else {
            self.window.sidebar_visible = !self.window.sidebar_visible;
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
        if self.selected.id == Some(id) || self.sessions.removed.contains(&id) {
            return;
        }
        self.save_title(cx);
        self.save_draft(cx);
        self.close_reply_preview(window, cx);
        self.window.usage_details = None;
        self.selected.select(id);
        self.navigation.restore.on_connect = false;
        self.remember_thread();
        self.composer.commands.dismissed = false;
        if let Some(session) = self
            .sessions
            .entries
            .iter()
            .find(|session| session.id == id)
        {
            self.selected.agent = session.agent.to_string();
            self.navigation.selected_directory =
                session.cwd.as_deref().map(navigation::groups::normalize);
            self.navigation
                .collapsed
                .remove(&self.navigation.selected_directory);
        }
        if restore_draft {
            self.composer.expanded = false;
            self.sync_composer_mode(cx);
            let draft = self.sessions.drafts.get(&id).cloned().unwrap_or_default();
            self.composer.switch_target(draft, window, cx);
            self.focus_input(window, cx);
        }
        self.command(
            Command::Watch {
                session: id,
                epoch: self.selected.epoch,
            },
            cx,
        );
        cx.notify();
    }

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_thread(self.navigation.selected_directory.clone(), window, cx);
    }

    fn new_thread(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.detach_thread(directory, window, cx);
        self.navigation
            .drafts
            .metadata
            .entry(self.navigation.selected_directory.clone())
            .or_default();
        let draft = self
            .navigation
            .drafts
            .contents
            .get(&self.navigation.selected_directory)
            .cloned()
            .unwrap_or_default();
        self.composer.expanded = !self.window.compact;
        self.sync_composer_mode(cx);
        self.composer.replace(draft, window, cx);
        self.focus_input(window, cx);
        cx.notify();
    }

    fn detach_thread(
        &mut self,
        directory: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_title(cx);
        self.navigation.archived = false;
        self.save_draft(cx);
        self.close_reply_preview(window, cx);
        self.selected.detach();
        self.composer.invalidate_target();
        self.navigation.restore.on_connect = false;
        self.remember_thread();
        self.composer.commands.dismissed = false;
        self.navigation.selected_directory = directory;
        if let Some(directory) = &self.navigation.selected_directory
            && !self.navigation.directories.contains(directory)
        {
            self.navigation.directories.push(directory.clone());
        }
        self.navigation
            .collapsed
            .remove(&self.navigation.selected_directory);
        self.command(Command::Detach, cx);
    }

    fn current(&self) -> Option<&SessionMeta> {
        self.selected
            .timeline
            .conversation
            .as_ref()
            .map(|state| &state.session)
            .or_else(|| {
                self.sessions
                    .entries
                    .iter()
                    .find(|session| Some(session.id) == self.selected.id)
            })
    }

    fn agent_icon(
        &self,
        element_id: impl Into<SharedString>,
        agent: &str,
        cx: &App,
    ) -> crate::agents::icon::AgentIcon {
        let uri = self
            .backend
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.read(cx).icon(self.backend.server, agent))
            .unwrap_or_default();
        gds_uim::el! {
            crate::agents::icon::AgentIcon(element_id, uri) {
                size: AGENT_ICON_SIZE.px;
            }
        }
    }

    fn working(&self) -> bool {
        self.current().is_some_and(|session| {
            self.sessions.healthy(session.id) && session.status == SessionStatus::Working
        })
    }

    fn can_send(&self, cx: &App) -> bool {
        if self.composer.paste_pending {
            return false;
        }
        if self.selected.approval.error.is_some() {
            return false;
        }
        let ready = self.selected.id.is_some()
            && self.selected.stream == StreamState::Live
            && self.current().is_some_and(|session| {
                matches!(session.status, SessionStatus::Idle | SessionStatus::Working)
                    && self.sessions.healthy(session.id)
                    && !self.archive_blocked(session.id)
                    && !self.sessions.pending.cancelling.contains(&session.id)
                    && !self.sending(session.id)
                    && !self.sessions.pending.configuring.contains(&session.id)
                    && !self
                        .sessions
                        .pending
                        .loading_attachments
                        .contains(&session.id)
            });
        self.connection_status() == status::ConnectionStatus::Connected
            && !self.selected.creating
            && ready
            && self.attachment_error(cx).is_none()
            && !self.composer.input.read(cx).value().trim().is_empty()
    }

    fn prepare_session(&mut self, cx: &mut Context<Self>) {
        if !self.backend.connected
            || self.navigation.archived
            || self.selected.id.is_some()
            || self.selected.creating
            || self.selected.waiting_for_agent_selection
            || self.selected.authentication.required.is_some()
            || self.selected.agent.is_empty()
        {
            return;
        }
        let cwd = match self
            .navigation
            .selected_directory
            .as_deref()
            .map(bas::service::utf8_cwd)
            .transpose()
        {
            Err(_) => {
                self.selected.error = Some(crate::i18n::tr!("workspace-encoding").into());
                cx.notify();
                return;
            }
            Ok(path) => path.unwrap_or_default().to_owned(),
        };
        self.save_title(cx);
        self.selected.creating = true;
        self.command(
            Command::Create(connection::CreateSession {
                epoch: self.selected.epoch,
                agent: self.selected.agent.clone(),
                title: self
                    .navigation
                    .drafts
                    .metadata
                    .get(&self.navigation.selected_directory)
                    .and_then(|draft| draft.title.clone())
                    .unwrap_or_default(),
                cwd,
            }),
            cx,
        );
        cx.notify();
    }

    fn can_select_agent(&self) -> bool {
        if self.selected.creating || self.composer.paste_pending || self.navigation.archived {
            return false;
        }
        let Some(id) = self.selected.id else {
            return self.current().is_none();
        };
        !self.thread_has_user_prompt()
            && self.connection_status() == status::ConnectionStatus::Connected
            && self.current().is_some_and(|session| {
                session.status == SessionStatus::Idle && session.active_turn.is_none()
            })
            && self
                .selected
                .timeline
                .conversation
                .as_ref()
                .is_some_and(|conversation| {
                    conversation.cursor >= conversation.session.revision.raw()
                })
            && !self.archive_blocked(id)
            && !self.sessions.pending.deleting.contains(&id)
            && !self.sessions.pending.configuring.contains(&id)
            && !self.sessions.pending.cancelling.contains(&id)
            && !self.sessions.pending.loading_attachments.contains(&id)
            && !self
                .sessions
                .outboxes
                .get(&id)
                .is_some_and(outbox::SessionOutbox::pending)
            && self.selected.approval.panel.is_empty()
            && self.selected.elicitations.is_empty()
    }

    pub(super) fn thread_has_user_prompt(&self) -> bool {
        // Unknown history is treated as started until a complete replay proves otherwise.
        self.selected
            .id
            .is_some_and(|id| !self.sessions.unstarted.contains(&id))
    }

    fn record_user_prompt(&mut self, session: SessionId) {
        self.sessions.record_prompt(session);
    }

    fn observe_user_prompt(&mut self, event: &bas::model::Event) {
        if matches!(
            event.kind,
            bas::model::EventKind::TurnStarted { .. }
                | bas::model::EventKind::TurnFinished { .. }
                | bas::model::EventKind::TurnInterrupted { .. }
                | bas::model::EventKind::StatusChanged {
                    status: SessionStatus::Working
                }
                | bas::model::EventKind::Acp(bas::acp::SessionUpdate::UserMessageChunk(_))
        ) {
            self.record_user_prompt(event.session);
        }
    }

    fn start_agent_chat(&mut self, agent: String, window: &mut Window, cx: &mut Context<Self>) {
        self.selected.agent = agent;
        self.new_thread(self.navigation.selected_directory.clone(), window, cx);
        self.selected.waiting_for_agent_selection = false;
        self.prepare_session(cx);
    }

    fn select_agent(&mut self, agent: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_select_agent() {
            return;
        }
        if self.selected.id.is_some() && self.selected.agent == agent {
            self.focus_input(window, cx);
            return;
        }
        self.detach_thread(self.navigation.selected_directory.clone(), window, cx);
        self.selected.agent = agent;
        self.selected.waiting_for_agent_selection = false;
        self.prepare_session(cx);
        self.focus_input(window, cx);
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.measure_composer(cx);
        self.sync_foreground(window);
        self.sync_reply_visibility(window, cx);
        if self.window.compact {
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
        .unwrap_or_else(|| crate::i18n::tr!("session-new").into())
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
    gds_uim::el! {
        #div {
            text_size: crate::ui::SMALL;
            text_color: $cx.theme().muted_foreground;
        } [$text.into()]
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
