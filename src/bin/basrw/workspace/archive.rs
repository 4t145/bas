use super::*;

impl Workspace {
    pub(super) fn archive_blocked(&self, session: SessionId) -> bool {
        self.archive_pending.contains(&session)
            || self.delete_pending.contains(&session)
            || self
                .sessions
                .iter()
                .any(|meta| meta.id == session && meta.archived)
            || self.conversation.as_ref().is_some_and(|conversation| {
                conversation.session.id == session && conversation.session.archived
            })
    }

    pub(super) fn pause_archived(&mut self) {
        for meta in &self.sessions {
            if meta.archived
                && let Some(outbox) = self.outboxes.get_mut(&meta.id)
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
        if !self.connected
            || self.archive_pending.contains(&session)
            || self.delete_pending.contains(&session)
        {
            return;
        }
        self.archive_pending.insert(session);
        // Pending blocks dispatch without changing the user's pause/edit state.
        // Only authoritative archived metadata pauses the queue permanently.
        if !self.connection.send(Command::Archive { session, archived }) {
            self.archive_pending.remove(&session);
            self.operation_error(
                "归档失败",
                "连接任务已停止；请重新打开 BASR 后重试。草稿和队列已保留。".into(),
                cx,
            );
        }
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
        layout::icon_button(
            SharedString::from(format!("{location}-{id}")),
            if archived {
                IconName::ArchiveRestore
            } else {
                IconName::Archive
            },
        )
        .accessibility_label(if archived {
            "恢复 thread"
        } else {
            "归档 thread"
        })
        .tooltip(if archived {
            "恢复到普通列表；队列保持暂停"
        } else {
            "保留历史并从普通列表隐藏；不会停止当前任务"
        })
        .disabled(
            !self.connected
                || self.archive_pending.contains(&id)
                || self.delete_pending.contains(&id),
        )
        .loading(self.archive_pending.contains(&id))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.set_archived(id, !archived, cx);
        }))
    }

    pub(super) fn visible_sessions(&self) -> Vec<SessionMeta> {
        self.sessions
            .iter()
            .filter(|meta| meta.archived == self.archived_list)
            .cloned()
            .collect()
    }
}
