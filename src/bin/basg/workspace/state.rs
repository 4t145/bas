use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    rc::Rc,
};

use bas::model::{SessionId, SessionMeta};
use bas::service::tls::ConnectionOptions as Target;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, FollowMode, ListState, Pixels, Size, Subscription, Task,
    Window,
    component::{input::*, *},
    prelude::*,
};

use crate::{
    attachment::{Attachment, Draft},
    connection::{AgentSummary, Connection, Failure, StreamState},
    conversation::Conversation,
    theme,
};

use super::{
    Workspace, composer, images, initial_size, input_layout, markdown, menus, navigation, outbox,
    rename, reply, runtime,
};

/// Window presentation and popups, not the lifetime of a selected thread.
pub(super) struct WindowState {
    pub(super) handle: AnyWindowHandle,
    pub(super) presented: bool,
    pub(super) menu: Option<menus::OpenMenu>,
    pub(super) usage_details: Option<SessionId>,
    pub(super) reply_preview: Option<reply::Preview>,
    pub(super) reply_hover: Option<Task<()>>,
    pub(super) activity_hover: Option<Task<()>>,
    pub(super) sidebar_visible: bool,
    pub(super) compact: bool,
    pub(super) output_open: bool,
    pub(super) size: Size<Pixels>,
    pub(super) popup_size: Option<Size<Pixels>>,
}

impl WindowState {
    pub(super) fn new(handle: AnyWindowHandle, compact: bool) -> Self {
        Self {
            handle,
            presented: true,
            menu: None,
            usage_details: None,
            reply_preview: None,
            reply_hover: None,
            activity_hover: None,
            sidebar_visible: !compact,
            compact,
            output_open: false,
            size: initial_size(false),
            popup_size: None,
        }
    }
}

/// One daemon connection, including the task delivering its updates.
pub(super) struct BackendState {
    pub(super) connection: Connection,
    pub(super) server: Target,
    pub(super) foreground: Option<SessionId>,
    pub(super) agents: Vec<AgentSummary>,
    pub(super) metadata: Option<Entity<crate::agents::metadata::Metadata>>,
    pub(super) connected: bool,
    pub(super) error: Option<Failure>,
    pub(super) _events: Task<()>,
}

impl BackendState {
    pub(super) fn new(
        connection: Connection,
        server: Target,
        metadata: Option<Entity<crate::agents::metadata::Metadata>>,
        events: Task<()>,
    ) -> Self {
        Self {
            connection,
            server,
            foreground: None,
            agents: Vec::new(),
            metadata,
            connected: false,
            error: None,
            _events: events,
        }
    }

    pub(super) fn connection_ready(&mut self) {
        self.connected = true;
        self.error = None;
    }

    pub(super) fn connection_lost(&mut self, error: Failure) {
        self.connected = false;
        self.error = Some(error);
    }
}

/// Sidebar selection, directory drafts, and persisted navigation preferences.
pub(super) struct NavigationState {
    pub(super) search: Entity<InputState>,
    pub(super) directories: Vec<PathBuf>,
    pub(super) selected_directory: Option<PathBuf>,
    pub(super) collapsed: HashSet<Option<PathBuf>>,
    pub(super) hidden: HashSet<Option<PathBuf>>,
    pub(super) removing: HashMap<Option<PathBuf>, navigation::removal::Removal>,
    pub(super) view: navigation::View,
    pub(super) pinned: HashSet<SessionId>,
    pub(super) bookmarks: HashSet<SessionId>,
    pub(super) restore: RestoreState,
    pub(super) drafts: DirectoryDrafts,
    pub(super) picker: FolderPicker,
    pub(super) _clock: Option<Task<()>>,
}

impl NavigationState {
    pub(super) fn new(search: Entity<InputState>, cwd: PathBuf) -> Self {
        let directory = navigation::groups::normalize(&cwd);
        Self {
            search,
            directories: vec![directory.clone()],
            selected_directory: Some(directory),
            collapsed: HashSet::new(),
            hidden: HashSet::new(),
            removing: HashMap::new(),
            view: navigation::View::Workspaces,
            pinned: HashSet::new(),
            bookmarks: HashSet::new(),
            restore: RestoreState::default(),
            drafts: DirectoryDrafts::default(),
            picker: FolderPicker::default(),
            _clock: None,
        }
    }

    pub(super) fn attach_restore(&mut self, store: Option<navigation::restore::Store>) {
        self.bookmarks = store
            .as_ref()
            .map(navigation::restore::Store::bookmarks)
            .unwrap_or_default();
        self.hidden = store
            .as_ref()
            .map(navigation::restore::Store::hidden_workspaces)
            .unwrap_or_default();
        self.pinned = store
            .as_ref()
            .map(navigation::restore::Store::pinned)
            .unwrap_or_default();
        self.restore.store = store;
        self.restore.on_connect = true;
        self.restore.read_save.take();
    }
}

#[derive(Default)]
pub(super) struct RestoreState {
    pub(super) store: Option<navigation::restore::Store>,
    pub(super) on_connect: bool,
    pub(super) read_save: Option<Task<()>>,
    pub(super) pending_save: Option<navigation::restore::PendingSave>,
}

#[derive(Default)]
pub(super) struct DirectoryDrafts {
    pub(super) contents: HashMap<Option<PathBuf>, Draft>,
    pub(super) metadata: HashMap<Option<PathBuf>, navigation::draft_row::Metadata>,
}

impl DirectoryDrafts {
    pub(super) fn remove(&mut self, directory: &Option<PathBuf>) {
        self.contents.remove(directory);
        self.metadata.remove(directory);
    }
}

#[derive(Default)]
pub(super) struct FolderPicker {
    pub(super) pending: bool,
    pub(super) waiting_activation: bool,
    pub(super) session: Option<(SessionId, u64)>,
}

/// State shared by foreground and background sessions; selection never resets it.
#[derive(Default)]
pub(super) struct SessionStore {
    pub(super) entries: Vec<SessionMeta>,
    pub(super) runtimes: runtime::Runtimes,
    pub(super) observed: HashSet<SessionId>,
    pub(super) removed: HashSet<SessionId>,
    pub(super) drafts: HashMap<SessionId, Draft>,
    pub(super) outboxes: HashMap<SessionId, outbox::SessionOutbox>,
    pub(super) unstarted: HashSet<SessionId>,
    pub(super) prompted: HashSet<SessionId>,
    pub(super) seen_replies: HashMap<SessionId, u64>,
    pub(super) pending: PendingOperations,
    pub(super) errors: HashMap<SessionId, String>,
    pub(super) history: super::timeline::Cache,
}

impl SessionStore {
    pub(super) fn healthy(&self, id: SessionId) -> bool {
        !self.removed.contains(&id) && self.runtimes.failure(id).is_none()
    }

    pub(super) fn is_draft(&self, id: SessionId) -> bool {
        self.unstarted.contains(&id) && !self.prompted.contains(&id)
    }

    pub(super) fn record_prompt(&mut self, id: SessionId) {
        if self.removed.contains(&id) {
            return;
        }
        self.prompted.insert(id);
        self.unstarted.remove(&id);
    }

    pub(super) fn mark_removed(&mut self, id: SessionId) {
        self.runtimes.remove(id);
        self.errors.remove(&id);
        self.removed.insert(id);
    }

    pub(super) fn discard_content(&mut self, id: SessionId) {
        self.drafts.remove(&id);
        self.outboxes.remove(&id);
        self.unstarted.remove(&id);
        self.prompted.remove(&id);
        self.history.remove(id);
    }

    pub(super) fn observe_ids(&mut self, ids: HashSet<SessionId>) -> Vec<SessionId> {
        let disappeared = self.observed.difference(&ids).copied().collect();
        self.observed = ids;
        disappeared
    }
}

#[derive(Default)]
pub(super) struct PendingOperations {
    pub(super) configuring: HashSet<SessionId>,
    pub(super) cancelling: HashSet<SessionId>,
    pub(super) archiving: HashSet<SessionId>,
    pub(super) deleting: HashSet<SessionId>,
    pub(super) closing: HashSet<SessionId>,
    pub(super) restarting: HashSet<SessionId>,
    pub(super) loading_attachments: HashSet<SessionId>,
}

impl PendingOperations {
    pub(super) fn forget(&mut self, id: SessionId) {
        self.configuring.remove(&id);
        self.cancelling.remove(&id);
        self.archiving.remove(&id);
        self.deleting.remove(&id);
        self.closing.remove(&id);
        self.restarting.remove(&id);
        self.loading_attachments.remove(&id);
    }
}

/// The selected page and its watch epoch, independent of in-flight session operations.
pub(super) struct SelectedThread {
    pub(super) id: Option<SessionId>,
    pub(super) agent: String,
    pub(super) epoch: u64,
    pub(super) stream: StreamState,
    pub(super) creating: bool,
    pub(super) waiting_for_agent_selection: bool,
    pub(super) error: Option<String>,
    pub(super) approval: ApprovalState,
    pub(super) authentication: AuthenticationState,
    pub(super) elicitations: BTreeMap<String, ObservedPanel<crate::elicitation::Panel>>,
    pub(super) timeline: TimelineState,
}

impl SelectedThread {
    pub(super) fn new(list: ListState) -> Self {
        Self {
            id: None,
            agent: String::new(),
            epoch: 0,
            stream: StreamState::Loading,
            creating: false,
            waiting_for_agent_selection: false,
            error: None,
            approval: ApprovalState::default(),
            authentication: AuthenticationState::default(),
            elicitations: BTreeMap::new(),
            timeline: TimelineState::new(list),
        }
    }

    pub(super) fn select(&mut self, id: SessionId) {
        self.clear_transients();
        self.id = Some(id);
        self.waiting_for_agent_selection = false;
        self.reload();
        self.timeline.list.set_follow_mode(FollowMode::Tail);
    }

    pub(super) fn detach(&mut self) {
        self.clear_transients();
        self.id = None;
        self.waiting_for_agent_selection = true;
        self.epoch += 1;
    }

    pub(super) fn reload(&mut self) {
        self.epoch += 1;
        self.stream = StreamState::Loading;
    }

    pub(super) fn awaiting_history(&self) -> bool {
        self.id.is_some() && self.timeline.conversation.is_none()
    }

    fn clear_transients(&mut self) {
        self.creating = false;
        self.error = None;
        self.approval.clear();
        self.authentication.clear();
        self.elicitations.clear();
        self.timeline.clear_thread();
    }
}

#[derive(Default)]
pub(super) struct ApprovalState {
    pub(super) panel: crate::workspace::interaction::approval::Panel,
    pub(super) error: Option<String>,
}

impl ApprovalState {
    pub(super) fn clear(&mut self) {
        self.panel.clear();
        self.error = None;
    }
}

#[derive(Default)]
pub(super) struct AuthenticationState {
    pub(super) required: Option<String>,
    pub(super) panel: Option<ObservedPanel<crate::authentication::Authentication>>,
    pub(super) load: Option<Task<()>>,
}

impl AuthenticationState {
    pub(super) fn clear(&mut self) {
        self.required = None;
        self.panel = None;
        self.load = None;
    }
}

/// A transient panel owns its observers; removing it also unsubscribes its callbacks.
pub(super) struct ObservedPanel<T: 'static> {
    pub(super) entity: Entity<T>,
    _subscriptions: Vec<Subscription>,
}

impl<T: 'static> ObservedPanel<T> {
    pub(super) fn new(entity: Entity<T>, subscriptions: Vec<Subscription>) -> Self {
        Self {
            entity,
            _subscriptions: subscriptions,
        }
    }
}

/// Rendered history. Replaying a snapshot clears caches, not thread-local expansion.
pub(super) struct TimelineState {
    pub(super) conversation: Option<Conversation>,
    pub(super) history: Option<bas::service::HistoryWindow>,
    pub(super) history_weight: crate::connection::HistoryWeight,
    // Transient / failed-runtime presentation can outlive its committed window.
    pub(super) presentation_bytes: usize,
    pub(super) presentation_meta_bytes: usize,
    pub(super) loading_before: Option<u64>,
    pub(super) older_error: Option<Failure>,
    pub(super) restore: Option<super::timeline::Position>,
    pub(super) markdown: RefCell<HashMap<String, markdown::Document>>,
    pub(super) images: RefCell<HashMap<String, images::Entry>>,
    pub(super) expanded: HashSet<String>,
    pub(super) bounds: Rc<RefCell<HashMap<String, gpui_kit::Bounds<Pixels>>>>,
    pub(super) list: ListState,
}

impl TimelineState {
    pub(super) fn new(list: ListState) -> Self {
        Self {
            conversation: None,
            history: None,
            history_weight: Default::default(),
            presentation_bytes: 0,
            presentation_meta_bytes: 0,
            loading_before: None,
            older_error: None,
            restore: None,
            markdown: RefCell::new(HashMap::new()),
            images: RefCell::new(HashMap::new()),
            expanded: HashSet::new(),
            bounds: Rc::default(),
            list,
        }
    }

    pub(super) fn clear_caches(&mut self) {
        self.markdown.borrow_mut().clear();
        self.images.borrow_mut().clear();
    }

    pub(super) fn clear_thread(&mut self) {
        self.clear_caches();
        self.conversation = None;
        self.history = None;
        self.history_weight = Default::default();
        self.presentation_bytes = 0;
        self.presentation_meta_bytes = 0;
        self.loading_before = None;
        self.older_error = None;
        self.restore = None;
        self.expanded.clear();
        self.bounds.borrow_mut().clear();
        self.list.reset(0);
    }
}

/// The visible composition; its generation guards asynchronous editor work.
pub(super) struct ComposerState {
    pub(super) input: Entity<EditorState>,
    pub(super) expanded: bool,
    pub(super) available_height: Cell<f32>,
    pub(super) mode: composer::Mode,
    pub(super) layout: input_layout::InputLayout,
    pub(super) rows: usize,
    pub(super) content_left: Cell<Pixels>,
    pub(super) mentions: MentionState,
    pub(super) attachments: Vec<Attachment>,
    pub(super) paste_pending: bool,
    pub(super) generation: u64,
    pub(super) commands: CommandMenuState,
}

impl ComposerState {
    pub(super) fn draft(&self, cx: &App) -> Draft {
        Draft {
            text: self.input.read(cx).value().to_string(),
            attachments: self.attachments.clone(),
        }
    }

    pub(super) fn matches(&self, draft: &Draft, cx: &App) -> bool {
        draft.matches(self.input.read(cx).value().as_ref(), &self.attachments)
    }

    /// Replace content within the current target without invalidating its asynchronous work.
    pub(super) fn replace(
        &mut self,
        draft: Draft,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        self.attachments = draft.attachments;
        self.input
            .update(cx, |input, cx| input.set_value(draft.text, window, cx));
    }

    /// Switching drafts invalidates work captured for the previous composition.
    pub(super) fn switch_target(
        &mut self,
        draft: Draft,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        self.invalidate_target();
        self.replace(draft, window, cx);
    }

    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>, compact: bool) -> Self {
        let mode = if compact {
            cx.global::<theme::Appearance>().composer_enter_behavior
        } else {
            composer::Mode::Newline
        };
        let input = cx.new(|cx| {
            EditorState::new(window, cx)
                .placeholder(crate::i18n::tr!("composer-placeholder"))
                .language("markdown")
                .line_number(false)
                .folding(false)
                .auto_close(false)
                .smart_indent(false)
                .soft_wrap(true)
                .scroll_beyond_last_line(Some(0))
                .submit_on_enter(mode == composer::Mode::Send)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let decorations = input.update(cx, |input, cx| {
            input.create_decorations_collection(Vec::new(), cx)
        });
        Self {
            input,
            expanded: !compact,
            available_height: Cell::new(620.),
            mode,
            layout: input_layout::InputLayout::new(
                gpui_kit::font(cx.theme().font_family.clone()),
                theme::content_font_size(cx),
            ),
            rows: 1,
            content_left: Cell::new(gpui_kit::component::Size::Medium.input_px()),
            mentions: MentionState {
                decorations,
                decorated: Vec::new(),
                selection: 0..0,
            },
            attachments: Vec::new(),
            paste_pending: false,
            generation: 0,
            commands: CommandMenuState::default(),
        }
    }

    pub(super) fn invalidate_target(&mut self) {
        self.generation += 1;
    }
}

pub(super) struct MentionState {
    pub(super) decorations: TextDecorationCollection,
    pub(super) decorated: Vec<TextDecoration>,
    pub(super) selection: Range<usize>,
}

#[derive(Default)]
pub(super) struct CommandMenuState {
    pub(super) selection: usize,
    pub(super) dismissed: bool,
}

/// One title editor plus requests that may finish after switching threads.
pub(super) struct RenameState {
    pub(super) input: Entity<InputState>,
    pub(super) edit: Option<rename::Edit>,
    pub(super) pending: HashMap<SessionId, String>,
    pub(super) retries: HashMap<SessionId, String>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl RenameState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            input: cx.new(|cx| {
                InputState::new(window, cx).placeholder(crate::i18n::tr!("session-title"))
            }),
            edit: None,
            pending: HashMap::new(),
            retries: HashMap::new(),
            _subscriptions: Vec::new(),
        }
    }

    pub(super) fn forget(&mut self, id: SessionId) {
        self.pending.remove(&id);
        self.retries.remove(&id);
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| edit.target == rename::Target::Thread(id))
        {
            self.edit = None;
        }
    }
}

#[cfg(test)]
mod tests;
