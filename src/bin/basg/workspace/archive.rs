use super::*;
use gds_uim::el;

impl Workspace {
    pub(super) fn archive_blocked(&self, session: SessionId) -> bool {
        self.sessions.pending.archiving.contains(&session)
            || self.sessions.pending.deleting.contains(&session)
            || self
                .sessions
                .entries
                .iter()
                .any(|meta| meta.id == session && meta.archived)
            || self
                .selected
                .timeline
                .conversation
                .as_ref()
                .is_some_and(|conversation| {
                    conversation.session.id == session && conversation.session.archived
                })
    }

    pub(super) fn pause_archived(&mut self) {
        for meta in &self.sessions.entries {
            if meta.archived
                && let Some(outbox) = self.sessions.outboxes.get_mut(&meta.id)
            {
                outbox.pause();
            }
        }
    }

    pub(super) fn set_archived(
        &mut self,
        session: SessionId,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.backend.connected
            || self.sessions.pending.archiving.contains(&session)
            || self.sessions.pending.deleting.contains(&session)
        {
            return;
        }
        self.sessions.pending.archiving.insert(session);
        // Pending blocks dispatch without changing the user's pause/edit state.
        // Only authoritative archived metadata pauses the queue permanently.
        self.command(Command::Archive { session, archived }, cx);
        cx.notify();
    }

    pub(super) fn archive_button(
        &self,
        session: &SessionMeta,
        location: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        let id = session.id;
        let archived = session.archived;
        let icon = if archived {
            IconName::ArchiveRestore
        } else {
            IconName::Archive
        };
        let label = if archived {
            crate::i18n::tr!("session-restore")
        } else {
            crate::i18n::tr!("session-archive")
        };
        el! {
            #layout::icon_button($SharedString::from(format!("{location}-{id}")), icon) {
                accessibility_label: label;
                tooltip: label;
                disabled: $!self.backend.connected
                    || self.sessions.pending.archiving.contains(&id)
                    || self.sessions.pending.deleting.contains(&id);
                loading: $self.sessions.pending.archiving.contains(&id);
                on_click: $cx.listener(move |this, _, _, cx| {
                    this.on_archive_clicked(id, !archived, cx);
                });
            }
        }
    }

    fn on_archive_clicked(&mut self, id: SessionId, archived: bool, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.set_archived(id, archived, cx);
    }

    pub(super) fn visible_sessions(&self) -> Vec<SessionMeta> {
        self.sessions
            .entries
            .iter()
            .filter(|meta| meta.archived == self.navigation.archived)
            .cloned()
            .collect()
    }
}
