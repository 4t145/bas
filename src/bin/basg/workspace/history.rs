//! Conversation integration only; the history window owns discovery and import UI.

use super::*;
use bas::model::SessionSnapshot;

impl Workspace {
    pub(super) fn history_request(
        &self,
        agent: String,
        cx: &mut Context<Self>,
    ) -> crate::history::OpenRequest {
        let owner = cx.entity().downgrade();
        let parent = self.window.handle;
        crate::history::OpenRequest {
            server: self.backend.server,
            agents: self.backend.agents.clone(),
            agent: Some(agent),
            cwd: self
                .current()
                .and_then(|meta| meta.cwd.clone())
                .or_else(|| self.navigation.selected_directory.clone()),
            workspaces: self
                .navigation
                .directories
                .iter()
                .cloned()
                .chain(
                    self.sessions
                        .entries
                        .iter()
                        .filter_map(|meta| meta.cwd.clone()),
                )
                .collect(),
            open_session: std::rc::Rc::new(move |snapshot, cx| {
                parent
                    .update(cx, |_, window, cx| {
                        owner.update(cx, |this, cx| {
                            this.adopt_agent_session(snapshot, window, cx);
                            window.activate_window();
                        })
                    })
                    .is_ok_and(|result| result.is_ok())
            }),
        }
    }

    pub(super) fn open_agent_history(
        &mut self,
        agent: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        let request = self.history_request(agent, cx);
        if let Err(error) = crate::history::window::open(request, cx) {
            self.operation_error(
                crate::i18n::tr!("agent-history-title"),
                error.to_string(),
                cx,
            );
        }
    }

    fn adopt_agent_session(
        &mut self,
        snapshot: SessionSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = snapshot.meta.id;
        if self.sessions.removed.contains(&id) {
            return;
        }
        self.sessions.runtimes.observe(&snapshot, false);
        if let Some(meta) = self.sessions.entries.iter_mut().find(|meta| meta.id == id) {
            crate::conversation::merge_meta(meta, &snapshot.meta);
        } else {
            self.sessions.entries.insert(0, snapshot.meta);
        }
        self.sessions.record_prompt(id);
        self.select(id, window, cx);
        cx.notify();
    }

    pub(super) fn agent_session_closed(
        &mut self,
        id: SessionId,
        result: Result<SessionSnapshot, Failure>,
        cx: &mut Context<Self>,
    ) {
        self.sessions.pending.closing.remove(&id);
        match result {
            Ok(snapshot) => {
                self.sessions.runtimes.observe(&snapshot, false);
                if let Some(meta) = self.sessions.entries.iter_mut().find(|meta| meta.id == id) {
                    crate::conversation::merge_meta(meta, &snapshot.meta);
                }
                if let Some(conversation) = &mut self.selected.timeline.conversation {
                    conversation.merge_meta(&snapshot.meta);
                }
                if let Some(outbox) = self.sessions.outboxes.get_mut(&id) {
                    outbox.pause();
                }
            }
            Err(error) => self.session_operation_error(
                id,
                crate::i18n::tr!("agent-session-close"),
                error.presentation(),
                cx,
            ),
        }
        cx.notify();
    }

    pub(super) fn request_close_agent_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if self.sessions.pending.closing.contains(&id) {
            return;
        }
        let confirmation = crate::application::Confirmation {
            title: crate::i18n::tr!("agent-session-close").into(),
            message: crate::i18n::tr!("agent-session-close-confirm").into(),
            confirm_label: crate::i18n::tr!("agent-session-close").into(),
        };
        let owner = cx.entity().downgrade();
        if let Err(error) = crate::application::open_confirmation(
            confirmation,
            move |cx| {
                let _ = owner.update(cx, |this, cx| {
                    if this.sessions.entries.iter().any(|meta| meta.id == id)
                        && !this.sessions.removed.contains(&id)
                        && this.sessions.pending.closing.insert(id)
                    {
                        this.command(Command::CloseSession(id), cx);
                    }
                });
            },
            cx,
        ) {
            self.operation_error(
                crate::i18n::tr!("agent-session-close"),
                error.to_string(),
                cx,
            );
        }
    }
}
