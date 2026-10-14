use gpui_kit::{Focusable, KeyDownEvent};

use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Target {
    Thread(SessionId),
    Draft(Option<PathBuf>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Location {
    Header,
    Sidebar,
}

pub(super) struct Edit {
    pub target: Target,
    pub location: Location,
    original: String,
}

impl Workspace {
    pub(super) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.selected.id.map_or_else(
            || Target::Draft(self.navigation.selected_directory.clone()),
            Target::Thread,
        );
        self.begin_title_edit(target, Location::Header, window, cx);
    }

    pub(super) fn begin_sidebar_rename(
        &mut self,
        session: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.window.compact || !self.window.sidebar_visible {
            return;
        }
        let query = self.navigation.search.read(cx).value().to_lowercase();
        let Some(meta) = self.sessions.entries.iter().find(|meta| {
            meta.id == session
                && meta.archived == self.navigation.archived
                && matches_query(meta, &query)
        }) else {
            return;
        };
        let directory = meta.cwd.as_deref().map(navigation::groups::normalize);
        if query.is_empty() && self.navigation.collapsed.contains(&directory) {
            return;
        }
        self.begin_title_edit(Target::Thread(session), Location::Sidebar, window, cx);
    }

    pub(super) fn begin_draft_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_title_edit(
            Target::Draft(self.navigation.selected_directory.clone()),
            Location::Sidebar,
            window,
            cx,
        );
    }

    fn begin_title_edit(
        &mut self,
        target: Target,
        location: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .renaming
            .edit
            .as_ref()
            .is_some_and(|edit| edit.target == target && edit.location == location)
        {
            self.renaming
                .input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        self.save_title(cx);
        let original = match &target {
            Target::Thread(session) => {
                if self.renaming.pending.contains_key(session) {
                    return;
                }
                let Some(meta) = self
                    .current()
                    .filter(|meta| meta.id == *session)
                    .or_else(|| {
                        self.sessions
                            .entries
                            .iter()
                            .find(|meta| meta.id == *session)
                    })
                else {
                    return;
                };
                title(meta)
            }
            Target::Draft(directory) => {
                if self.selected.creating || self.selected.id.is_some() {
                    return;
                }
                self.navigation
                    .drafts
                    .metadata
                    .get(directory)
                    .and_then(|draft| draft.title.clone())
                    .unwrap_or_else(|| crate::i18n::tr!("session-new").into())
            }
        };
        let attempt = match &target {
            Target::Thread(session) => self.renaming.retries.get(session).cloned(),
            Target::Draft(_) => None,
        }
        .unwrap_or_else(|| original.clone());

        // A fresh input identity prevents a queued Blur from an earlier editor
        // from submitting a replacement edit after a click changes its target.
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::tr!("session-title"))
                // Native text menus move focus and would detach their own editor on Blur.
                .context_menu(false)
                .default_value(attempt)
        });
        self.renaming.input = input.clone();
        self.renaming.edit = Some(Edit {
            target,
            location,
            original,
        });
        self.renaming._subscriptions = vec![
            cx.observe(&input, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, input, event, window, cx| {
                if input.entity_id() != this.renaming.input.entity_id()
                    || this.renaming.edit.is_none()
                {
                    return;
                }
                match event {
                    InputEvent::PressEnter { .. } => {
                        this.save_title(cx);
                        this.focus_input(window, cx);
                    }
                    InputEvent::Blur => this.save_title(cx),
                    _ => {}
                }
            }),
        ];
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn renaming_header(&self) -> bool {
        self.renaming
            .edit
            .as_ref()
            .is_some_and(|edit| edit.location == Location::Header)
    }

    pub(super) fn renaming_thread(&self, session: SessionId) -> bool {
        self.renaming.edit.as_ref().is_some_and(|edit| {
            edit.location == Location::Sidebar && edit.target == Target::Thread(session)
        })
    }

    pub(super) fn renaming_draft(&self) -> bool {
        self.renaming.edit.as_ref().is_some_and(|edit| {
            edit.location == Location::Sidebar
                && edit.target == Target::Draft(self.navigation.selected_directory.clone())
        })
    }

    pub(super) fn save_title(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.renaming.edit.take() else {
            return;
        };
        let attempt = self.renaming.input.read(cx).value().to_string();
        let title = attempt.trim().to_owned();
        if !title.is_empty() && title != edit.original.trim() {
            match edit.target {
                Target::Draft(directory) => {
                    let draft = self
                        .navigation
                        .drafts
                        .metadata
                        .entry(directory)
                        .or_default();
                    draft.title = Some(title);
                    draft.updated_at = bas::model::Timestamp::now();
                }
                Target::Thread(session) => {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        self.renaming.pending.entry(session)
                    {
                        entry.insert(attempt);
                        self.command(Command::Rename { session, title }, cx);
                    }
                }
            }
        } else if let Target::Thread(session) = edit.target {
            self.renaming.retries.remove(&session);
        }
        cx.notify();
    }

    pub(super) fn rename_completed(&mut self, session: SessionId, result: Result<(), String>) {
        let Some(attempt) = self.renaming.pending.remove(&session) else {
            return;
        };
        match result {
            Ok(()) => {
                self.renaming.retries.remove(&session);
                // The ordered event or catalog carries the title's revision.
            }
            Err(error) => {
                self.renaming.retries.insert(session, attempt);
                self.session_error(
                    session,
                    crate::i18n::tr!("session-rename-error", error = (error).to_string()),
                );
            }
        }
    }

    pub(super) fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Edit {
            target: Target::Thread(session),
            ..
        }) = self.renaming.edit.take()
        {
            self.renaming.retries.remove(&session);
        }
        self.focus_input(window, cx);
        cx.notify();
    }

    pub(super) fn render_title_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let input = self.renaming.input.clone();
        gds_uim::el! {
            #div {
                id: "title-editor"; flex_1; min_w_0; w_full;
                on_click: $|_, _, cx| cx.stop_propagation();
                on_mouse_down: gpui_kit::MouseButton::Right, $|_, _, cx| cx.stop_propagation();
                on_mouse_down_out: $cx.listener(
                move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    this.on_rename_mouse_down_out(&input, event, window, cx);
                },
                );
                on_key_down: $cx.listener(Self::on_rename_key_down);
            } [
                Input($&self.renaming.input) {
                    id: "rename-title-input"; small; w_full;
                    min_h: layout::ICON_BUTTON_SIZE.px;
                    max_h: layout::ICON_BUTTON_SIZE.px;
                }
            ]
        }
        .into_any_element()
    }

    fn on_rename_mouse_down_out(
        &mut self,
        input: &Entity<InputState>,
        event: &gpui_kit::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == gpui_kit::MouseButton::Left
            && input.read(cx).focus_handle(cx).is_focused(window)
        {
            window.blur(cx);
            // Commit before the click can remove the editor or its target row.
            self.save_title(cx);
        }
    }

    fn on_rename_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            self.cancel_rename(window, cx);
            cx.stop_propagation();
        }
    }
}
