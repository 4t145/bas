use super::*;

impl Workspace {
    pub(super) fn session_is_draft(&self, session: SessionId) -> bool {
        self.unstarted_sessions.contains(&session) && !self.prompted_sessions.contains(&session)
    }

    pub(super) fn discard_session_draft(
        &mut self,
        session: SessionId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session_is_draft(session)
            || self.archive_pending.contains(&session)
            || self.configuring.contains(&session)
            || self.loading_attachments.contains(&session)
            || (self.active == Some(session) && self.paste_pending)
            || self
                .outboxes
                .get(&session)
                .is_some_and(outbox::SessionOutbox::pending)
            || self.sessions.iter().any(|meta| {
                meta.id == session
                    && (meta.archived
                        || meta.status == SessionStatus::Working
                        || meta.active_turn.is_some())
            })
        {
            return;
        }
        self.delete_thread(session, cx);
    }

    fn delete_thread(&mut self, session: SessionId, cx: &mut Context<Self>) {
        if !self.connected
            || !self.sessions.iter().any(|meta| meta.id == session)
            || !self.delete_pending.insert(session)
        {
            return;
        }
        if !self.connection.send(Command::Delete(session)) {
            self.delete_pending.remove(&session);
            self.connected = false;
            self.operation_error(
                crate::i18n::tr!("session-delete-failed"),
                crate::i18n::tr!("session-delete-reopen").into(),
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn mark_thread_read(&mut self, session: SessionId, cx: &mut Context<Self>) {
        let cursor = self
            .sessions
            .iter()
            .find(|meta| meta.id == session)
            .map(|meta| meta.last_reply.raw())
            .unwrap_or(0);
        let replayed = self
            .conversation
            .as_ref()
            .filter(|conversation| conversation.session.id == session)
            .and_then(|conversation| conversation.last_reply)
            .map_or(0, |reply| reply.cursor);
        let seen = self.seen_replies.entry(session).or_default();
        *seen = (*seen).max(cursor).max(replayed);
        cx.notify();
    }

    pub(super) fn grouped_threads<'a>(
        &self,
        sessions: &'a [SessionMeta],
        query: &str,
    ) -> Vec<groups::Group<'a>> {
        let mut groups = groups::group_threads(sessions, &self.directories, query);
        for group in &mut groups {
            group
                .threads
                .sort_by_key(|session| !self.pinned.contains(&session.id));
        }
        groups
    }

    pub(super) fn toggle_pinned(&mut self, session: SessionId, cx: &mut Context<Self>) {
        let pinned = !self.pinned.contains(&session);
        if let Some(store) = &mut self.restore_store {
            if let Err(error) = store.set_pinned(session, pinned) {
                self.operation_error(
                    crate::i18n::tr!("session-pin-failed"),
                    error.to_string(),
                    cx,
                );
                return;
            }
            self.pinned = store.pinned();
        } else if pinned {
            self.pinned.insert(session);
        } else {
            self.pinned.remove(&session);
        }
        cx.notify();
    }

    pub(super) fn operation_error(&mut self, title: &str, message: String, cx: &mut Context<Self>) {
        self.error = Some(message.clone());
        if let Err(error) = crate::application::open_message(title, &message, cx) {
            tracing::error!(%error, %message, "cannot show operation failure");
        }
        cx.notify();
    }

    pub(super) fn request_delete(
        &mut self,
        session: SessionId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.delete_pending.contains(&session) || !self.connected {
            return;
        }
        let Some(meta) = self.sessions.iter().find(|meta| meta.id == session) else {
            return;
        };
        let confirmation = crate::application::Confirmation {
            title: crate::i18n::tr!("session-delete-title").into(),
            message: crate::i18n::tr!("session-delete-confirm", title = (title(meta)).to_string()),
            confirm_label: crate::i18n::tr!("action-delete-permanently").into(),
        };
        let view = cx.entity().downgrade();
        if let Err(error) = crate::application::open_confirmation(
            confirmation,
            move |cx| {
                let _ = view.update(cx, |this, cx| {
                    this.delete_thread(session, cx);
                });
            },
            cx,
        ) {
            self.operation_error(
                crate::i18n::tr!("session-delete-confirm-failed"),
                error.to_string(),
                cx,
            );
        }
    }
}
