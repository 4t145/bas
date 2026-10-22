//! Conversation integration only; the history window owns discovery and import UI.

use super::*;
use bas::model::SessionSnapshot;

impl Workspace {
    pub(super) fn retry_history(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected.id else {
            return;
        };
        if self.sessions.runtimes.failure(session).is_some() {
            if self.backend.connected && self.sessions.pending.restarting.insert(session) {
                self.command(Command::RestartSession(session), cx);
                cx.notify();
            }
            return;
        }
        if matches!(
            self.selected.stream,
            StreamState::Loading | StreamState::Live
        ) {
            return;
        }
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

    pub(super) fn history_retry_label(&self) -> &'static str {
        if self
            .selected
            .id
            .is_some_and(|id| self.sessions.runtimes.failure(id).is_some())
        {
            crate::i18n::tr!("session-persistence-retry")
        } else {
            crate::i18n::tr!("session-history-retry")
        }
    }

    pub(super) fn history_retry_disabled(&self) -> bool {
        self.selected.id.is_some_and(|id| {
            self.sessions.pending.restarting.contains(&id)
                || (self.sessions.runtimes.failure(id).is_some() && !self.backend.connected)
        })
    }

    pub(super) fn history_request(
        &self,
        agent: String,
        cx: &mut Context<Self>,
    ) -> crate::history::OpenRequest {
        let owner = cx.entity().downgrade();
        let parent = self.window.handle;
        crate::history::OpenRequest {
            server: self.backend.server.clone(),
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
                            if this.window.presented {
                                window.activate_window();
                            }
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
        if self.selected.id == Some(id) {
            // The stopped stream has ended. Reattach even when opening the same row,
            // without switching or clearing the user's unsent composition.
            self.selected.reload();
            self.command(
                Command::Watch {
                    session: id,
                    epoch: self.selected.epoch,
                },
                cx,
            );
        } else {
            self.select(id, window, cx);
        }
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
                self.selected.timeline.merge_meta(&snapshot.meta);
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

    pub(super) fn can_restart_agent_session(&self, id: SessionId) -> bool {
        self.backend.connected
            && self.sessions.healthy(id)
            && !self.sessions.removed.contains(&id)
            && !self.sessions.pending.restarting.contains(&id)
            && !self.sessions.pending.closing.contains(&id)
            && !self.sessions.pending.archiving.contains(&id)
            && !self.sessions.pending.deleting.contains(&id)
            && self.sessions.entries.iter().any(|meta| {
                meta.id == id
                    && meta.status == SessionStatus::Stopped
                    && !meta.archived
                    && (self
                        .backend
                        .agents
                        .iter()
                        .any(|agent| agent.id == meta.agent.as_str() && agent.kind == "embedded")
                        || meta.initialization.as_ref().is_some_and(|init| {
                            init.agent_capabilities.load_session
                                || init
                                    .agent_capabilities
                                    .session_capabilities
                                    .resume
                                    .is_some()
                        }))
            })
    }

    pub(super) fn request_restart_agent_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if !self.can_restart_agent_session(id) {
            return;
        }
        self.sessions.pending.restarting.insert(id);
        self.command(Command::RestartSession(id), cx);
        cx.notify();
    }

    pub(super) fn agent_session_restarted(
        &mut self,
        id: SessionId,
        result: Result<SessionSnapshot, Failure>,
        cx: &mut Context<Self>,
    ) {
        if !self.sessions.pending.restarting.remove(&id) {
            return;
        }
        match result {
            Ok(snapshot) => {
                let recovered = self.sessions.runtimes.failure(id).is_some();
                self.sessions.runtimes.observe(&snapshot, false);
                if let Some(meta) = self.sessions.entries.iter_mut().find(|meta| meta.id == id) {
                    crate::conversation::merge_meta(meta, &snapshot.meta);
                }
                if self.selected.id == Some(id) {
                    self.selected.timeline.merge_meta(&snapshot.meta);
                    if recovered {
                        self.command(Command::Refresh, cx);
                        return;
                    }
                    self.selected.reload();
                    self.command(
                        Command::Watch {
                            session: id,
                            epoch: self.selected.epoch,
                        },
                        cx,
                    );
                }
            }
            Err(error) => self.session_operation_error(
                id,
                crate::i18n::tr!("agent-session-start"),
                error.presentation(),
                cx,
            ),
        }
        cx.notify();
    }

    pub(super) fn request_close_agent_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if !self.backend.connected
            || self.sessions.pending.closing.contains(&id)
            || self.sessions.pending.restarting.contains(&id)
            || self.sessions.pending.archiving.contains(&id)
            || self.sessions.pending.deleting.contains(&id)
            || !self.sessions.entries.iter().any(|meta| {
                meta.id == id
                    && !meta.archived
                    && !matches!(meta.status, SessionStatus::Stopped | SessionStatus::Failed)
            })
        {
            return;
        }
        self.sessions.pending.closing.insert(id);
        self.command(Command::CloseSession(id), cx);
        cx.notify();
    }
}
