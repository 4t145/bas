//! Apply backend updates to shared session data and the selected thread.

use super::*;

impl Workspace {
    pub(super) fn update(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        let target = match &update {
            Update::HistoryMeta { session, .. } => Some(session.meta.id),
            Update::HistoryWindow { window, .. } => Some(window.session.meta.id),
            Update::Event { event, .. } => Some(event.session),
            Update::TrackedTurn { event } => Some(event.session),
            Update::ElicitationResponded { session, .. }
            | Update::PermissionResponded { session, .. }
            | Update::Configured { session, .. }
            | Update::Renamed { session, .. }
            | Update::Archived { session, .. }
            | Update::Deleted { session, .. }
            | Update::ClosedSession { session, .. }
            | Update::RestartedSession { session, .. }
            | Update::DirectoriesUpdated { session, .. }
            | Update::Submission { session, .. }
            | Update::TrackingFailed { session, .. }
            | Update::HistoryOverview { session, .. }
            | Update::HistoryLocation { session, .. }
            | Update::Cancelled { session, .. } => Some(*session),
            // Creation must still settle its pending attempt; catalogs filter each entry.
            // Session watch messages without an ID are invalidated by selection's epoch.
            Update::Created { .. }
            | Update::AgentHistory { .. }
            | Update::ImportedHistory { .. }
            | Update::DeletedAgentHistory { .. }
            | Update::Catalog(_)
            | Update::AuthenticationRequired { .. }
            | Update::OlderHistory { .. }
            | Update::Stream { .. }
            | Update::Permissions { .. }
            | Update::PermissionUnavailable { .. }
            | Update::Elicitations { .. }
            | Update::ElicitationUnavailable { .. } => None,
        };
        if target.is_some_and(|id| self.sessions.removed.contains(&id)) {
            return;
        }
        match update {
            // History discovery and mutations belong to the independent history window.
            Update::AgentHistory { .. }
            | Update::ImportedHistory { .. }
            | Update::DeletedAgentHistory { .. } => {}
            Update::ClosedSession { session, result } => {
                self.agent_session_closed(session, result, cx)
            }
            Update::RestartedSession { session, result } => {
                self.agent_session_restarted(session, result, cx)
            }
            Update::Elicitations {
                epoch,
                pending,
                completed_ids,
            } if epoch == self.selected.epoch && self.selected.id.is_some() => {
                self.sync_elicitations(pending, completed_ids, window, cx);
            }
            Update::ElicitationUnavailable { epoch, message } if epoch == self.selected.epoch => {
                if !self.selected.elicitations.is_empty() {
                    self.selected.error = Some(message.presentation());
                }
                self.selected.elicitations.clear();
            }
            Update::ElicitationResponded {
                session,
                request_id,
                result,
            } => {
                if self.selected.id == Some(session)
                    && let Err(error) = result
                    && let Some(panel) = self.selected.elicitations.get(&request_id)
                {
                    panel
                        .entity
                        .update(cx, |panel, cx| panel.failed(error.presentation(), cx));
                }
            }
            Update::DirectoriesUpdated { session, result } => {
                self.directories_updated(session, result, cx);
            }
            Update::Configured { session, result } => {
                self.configured(session, result.map_err(|error| error.presentation()));
            }
            Update::Catalog(result) => match result {
                Ok(mut catalog) => {
                    let failed_selection = self
                        .selected
                        .id
                        .filter(|id| self.sessions.runtimes.failure(*id).is_some());
                    if !self.backend.connected {
                        crate::agents::metadata::ensure(self.backend.server.clone(), cx);
                    }
                    self.backend.connection_ready();
                    // Session IDs are never reused. Older snapshots cannot revive
                    // a session whose removal has already been confirmed.
                    catalog
                        .sessions
                        .retain(|snapshot| !self.sessions.removed.contains(&snapshot.meta.id));
                    for snapshot in &catalog.sessions {
                        self.sessions.runtimes.observe(snapshot, true);
                    }
                    let mut sessions: Vec<_> = catalog
                        .sessions
                        .into_iter()
                        .map(|snapshot| snapshot.meta)
                        .collect();
                    let catalog_ids: HashSet<_> = sessions.iter().map(|meta| meta.id).collect();
                    // Catalog requests are serial, but create receipts can overtake them.
                    // Only disappearance after a prior catalog observation proves removal.
                    let removed = self.sessions.observe_ids(catalog_ids);
                    for meta in &mut sessions {
                        if let Some(old) =
                            self.sessions.entries.iter().find(|old| old.id == meta.id)
                        {
                            let incoming = std::mem::replace(meta, old.clone());
                            crate::conversation::merge_meta(meta, &incoming);
                        }
                        if let Some(conversation) = &self.selected.timeline.conversation
                            && conversation.session.id == meta.id
                        {
                            crate::conversation::merge_meta(meta, &conversation.session);
                        }
                        if meta.status == SessionStatus::Working || meta.active_turn.is_some() {
                            self.record_user_prompt(meta.id);
                        }
                    }
                    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
                    self.sessions.entries = sessions;
                    self.backend.agents = catalog.agents;
                    // An older catalog can arrive after choosing a newly installed agent.
                    if !self.selected.creating
                        && self.selected.id.is_none()
                        && self.selected.authentication.required.is_none()
                        && !self
                            .backend
                            .agents
                            .iter()
                            .any(|agent| agent.id == self.selected.agent)
                    {
                        self.selected.agent = self.new_thread_agent(cx);
                        self.selected.waiting_for_agent_selection = self.selected.agent.is_empty();
                        self.selected.authentication.required = None;
                    }
                    if let Some(id) = self
                        .selected
                        .timeline
                        .conversation
                        .as_ref()
                        .map(|conversation| conversation.session.id)
                        && let Some(meta) = self
                            .sessions
                            .entries
                            .iter()
                            .find(|session| session.id == id)
                    {
                        self.selected.timeline.merge_meta(meta);
                    }
                    self.sessions_removed(&removed, window, cx);
                    self.restore_thread(window, cx);
                    if let Some(id) = failed_selection
                        && self.selected.id == Some(id)
                        && self.sessions.healthy(id)
                    {
                        self.selected.error = None;
                        self.selected.reload();
                        self.command(
                            Command::Watch {
                                session: id,
                                epoch: self.selected.epoch,
                            },
                            cx,
                        );
                    }
                    if self.selected.error.is_none() {
                        self.prepare_session(cx);
                    }
                }
                Err(error) => {
                    self.backend.connection_lost(error);
                }
            },
            Update::Created { epoch, result } => {
                let current = epoch == self.selected.epoch && self.selected.id.is_none();
                if current {
                    self.selected.creating = false;
                }
                match result {
                    Ok(session) if self.sessions.removed.contains(&session.meta.id) => {
                        if current {
                            self.selected.error = Some(crate::i18n::tr!("session-deleted").into());
                        }
                    }
                    Ok(session) => {
                        self.sessions.runtimes.observe(&session, false);
                        let session = session.meta;
                        let id = session.id;
                        if current
                            && session.status != SessionStatus::Working
                            && session.active_turn.is_none()
                            && !self.sessions.prompted.contains(&id)
                        {
                            self.sessions.unstarted.insert(id);
                        }
                        if session.status == SessionStatus::Working || session.active_turn.is_some()
                        {
                            self.record_user_prompt(id);
                        }
                        if let Some(meta) =
                            self.sessions.entries.iter_mut().find(|meta| meta.id == id)
                        {
                            crate::conversation::merge_meta(meta, &session);
                        } else {
                            self.sessions.entries.insert(0, session);
                        }
                        if current {
                            self.sessions.drafts.insert(id, self.composer.draft(cx));
                            let directory = self.navigation.selected_directory.clone();
                            self.activate(id, false, window, cx);
                            self.navigation.drafts.remove(&directory);
                        }
                    }
                    Err(error) if current => {
                        self.selected.error = Some(crate::i18n::tr!(
                            "agent-init-error",
                            error = error.presentation()
                        ));
                    }
                    Err(_) => {}
                }
            }
            Update::AuthenticationRequired { epoch, agent } => {
                if epoch == self.selected.epoch
                    && self.selected.id.is_none()
                    && self.selected.agent == agent
                {
                    self.selected.creating = false;
                    self.selected.error = None;
                    self.selected.authentication.required = Some(agent);
                    self.load_authentication_panel(window, cx);
                }
            }
            Update::Renamed { session, result } => {
                self.rename_completed(session, result.map_err(|error| error.presentation()));
            }
            Update::Archived { session, result } => {
                self.sessions.pending.archiving.remove(&session);
                self.workspace_archive_completed(
                    session,
                    result.as_ref().is_ok_and(|snapshot| snapshot.meta.archived),
                );
                match result {
                    Ok(incoming) => {
                        if !self.sessions.removed.contains(&session) {
                            self.sessions.runtimes.observe(&incoming, false);
                        }
                        let incoming = incoming.meta;
                        // A newer restore snapshot must not erase this successful archive's
                        // queue pause, even when its metadata revision wins the merge.
                        if incoming.archived
                            && let Some(outbox) = self.sessions.outboxes.get_mut(&session)
                        {
                            outbox.pause();
                        }
                        // A late receipt must not resurrect a deleted catalog entry.
                        let mut reattach = false;
                        if let Some(meta) = self
                            .sessions
                            .entries
                            .iter_mut()
                            .find(|meta| meta.id == session)
                        {
                            reattach = self.selected.id == Some(session)
                                && meta.archived
                                && !incoming.archived
                                && incoming.revision >= meta.revision;
                            crate::conversation::merge_meta(meta, &incoming);
                            self.selected.timeline.merge_meta(meta);
                        }
                        if reattach {
                            self.selected.reload();
                            self.command(
                                Command::Watch {
                                    session,
                                    epoch: self.selected.epoch,
                                },
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        self.session_operation_error(
                            session,
                            crate::i18n::tr!("session-archive-update-failed"),
                            crate::i18n::tr!(
                                "session-operation-error",
                                error = error.presentation()
                            ),
                            cx,
                        );
                    }
                }
            }
            Update::Deleted { session, result } => {
                self.sessions.pending.deleting.remove(&session);
                match result {
                    Ok(()) => self.sessions_removed(&[session], window, cx),
                    Err(error) => {
                        self.session_operation_error(
                            session,
                            crate::i18n::tr!("session-delete-failed"),
                            crate::i18n::tr!(
                                "session-delete-error",
                                session = (session).to_string(),
                                error = error.presentation()
                            ),
                            cx,
                        );
                    }
                }
            }
            Update::Submission {
                session,
                submission,
                result,
            } => {
                if !self.apply_submission(session, submission, result, window, cx) {
                    return;
                }
            }
            Update::Cancelled { session, result } => {
                self.cancelled(session, result.map_err(|error| error.presentation()));
            }
            Update::HistoryWindow {
                epoch,
                window: history,
                cached,
            } if epoch == self.selected.epoch => {
                if !cached {
                    self.request_history_overview(history.cursor, cx);
                }
                if self.selected.timeline.navigation.detached {
                    if !cached {
                        self.sessions.runtimes.observe(&history.session, false);
                        self.selected.timeline.merge_meta(&history.session.meta);
                    }
                } else {
                    self.apply_history_window(history, cached, cx);
                }
            }
            Update::HistoryOverview {
                session,
                epoch,
                request,
                result,
            } if epoch == self.selected.epoch && self.selected.id == Some(session) => {
                self.history_overview_loaded(request, result, cx);
            }
            Update::HistoryLocation {
                session,
                epoch,
                request,
                result,
            } if epoch == self.selected.epoch && self.selected.id == Some(session) => {
                self.history_location_loaded(request, result, cx);
            }
            Update::OlderHistory {
                epoch,
                before,
                result,
            } if epoch == self.selected.epoch => {
                self.older_history_loaded(before, result, cx);
            }
            Update::HistoryMeta { epoch, session }
                if epoch == self.selected.epoch && self.selected.id == Some(session.meta.id) =>
            {
                self.sessions.runtimes.observe(&session, false);
                if let Some(meta) = self
                    .sessions
                    .entries
                    .iter_mut()
                    .find(|meta| meta.id == session.meta.id)
                {
                    crate::conversation::merge_meta(meta, &session.meta);
                }
                self.selected.timeline.merge_meta(&session.meta);
                if let Some(history) = &mut self.selected.timeline.history {
                    crate::conversation::merge_meta(&mut history.session.meta, &session.meta);
                    history.session.runtime = session.value.runtime;
                    self.selected
                        .timeline
                        .history_weight
                        .merge_meta(session.bytes);
                }
            }
            Update::Event { epoch, event } if epoch == self.selected.epoch => {
                let crate::connection::Prepared {
                    value: event,
                    bytes,
                } = event;
                if self.selected.id == Some(event.session)
                    && matches!(
                        event.kind,
                        bas::model::EventKind::TurnStarted { .. }
                            | bas::model::EventKind::Acp(
                                bas::acp::SessionUpdate::UserMessageChunk(_)
                            )
                    )
                {
                    self.request_history_overview(event.cursor.raw(), cx);
                }
                let browsing = self.selected.timeline.navigation.detached;
                if let Some(history) = &mut self.selected.timeline.history
                    && !browsing
                    && event.session == history.session.meta.id
                    && !event.is_transient()
                    && event.cursor.raw() > history.cursor
                {
                    history.cursor = event.cursor.raw();
                    history.session.meta.apply_event(&event);
                    history.events.push(event.clone());
                    self.selected.timeline.history_weight.append(bytes);
                }
                if !browsing
                    && self
                        .selected
                        .timeline
                        .conversation
                        .as_ref()
                        .is_some_and(|conversation| {
                            conversation.session.id == event.session
                                && !event.is_transient()
                                && event.cursor.raw() > conversation.cursor
                        })
                {
                    self.selected.timeline.presentation_bytes = self
                        .selected
                        .timeline
                        .presentation_bytes
                        .saturating_add(bytes.saturating_mul(2));
                }
                self.observe_user_prompt(&event);
                if let Some(meta) = self
                    .sessions
                    .entries
                    .iter_mut()
                    .find(|meta| meta.id == event.session)
                {
                    meta.apply_event(&event);
                    // Browsing an older slice must not freeze current controls.
                    // Text deltas do not change their state; avoid measuring the
                    // metadata snapshot for every streamed text fragment.
                    if browsing
                        && !matches!(
                            event.kind,
                            bas::model::EventKind::Acp(
                                bas::acp::SessionUpdate::UserMessageChunk(_)
                                    | bas::acp::SessionUpdate::AgentMessageChunk(_)
                                    | bas::acp::SessionUpdate::AgentThoughtChunk(_)
                            )
                        )
                    {
                        self.selected.timeline.merge_meta(meta);
                    }
                }
                if let Some(outbox) = self.sessions.outboxes.get_mut(&event.session) {
                    outbox.observe(&event);
                }
                if let Some(conversation) = &mut self.selected.timeline.conversation
                    && !browsing
                {
                    let old_count = conversation.rows.len();
                    let expanded_activity = conversation
                        .rows
                        .last()
                        .filter(|row| row.activity && row.items.len() == 1)
                        .map(|row| conversation.items[row.items.start].id())
                        .filter(|id| self.selected.timeline.expanded.contains(id));
                    let changed = conversation.apply(event);
                    let count = conversation.rows.len();
                    if let Some(id) = expanded_activity
                        && conversation
                            .rows
                            .get(old_count.saturating_sub(1))
                            .is_some_and(|row| row.items.len() > 1)
                    {
                        self.selected
                            .timeline
                            .expanded
                            .insert(format!("activity-{id}"));
                    }
                    if count > old_count {
                        self.selected
                            .timeline
                            .list
                            .splice(old_count..old_count, count - old_count);
                    }
                    if let Some(index) = changed.and_then(|index| conversation.row_for_item(index))
                    {
                        self.selected.timeline.list.remeasure_items(index..count);
                    }
                    if let Some(meta) = self
                        .sessions
                        .entries
                        .iter_mut()
                        .find(|meta| meta.id == conversation.session.id)
                    {
                        crate::conversation::merge_meta(meta, &conversation.session);
                    }
                }
            }
            Update::TrackedTurn { event } => {
                // Tracking skips intermediate metadata events. Only the complete
                // foreground stream or catalog may advance SessionMeta's revision.
                if let Some(outbox) = self.sessions.outboxes.get_mut(&event.session) {
                    outbox.observe(&event);
                }
            }
            Update::TrackingFailed {
                session,
                receipt,
                message,
            } => {
                if self
                    .sessions
                    .outboxes
                    .get_mut(&session)
                    .is_some_and(|outbox| outbox.tracking_failed(receipt))
                {
                    self.session_error(
                        session,
                        crate::i18n::tr!(
                            "turn-tracking-failed",
                            session = session.to_string(),
                            message = message.presentation()
                        ),
                    );
                }
            }
            Update::Stream { epoch, state } if epoch == self.selected.epoch => {
                self.selected.stream = state
            }
            Update::Permissions { epoch, pending } if epoch == self.selected.epoch => {
                self.selected.approval.error = None;
                self.selected.approval.panel.replace(pending);
                if self.window.compact && !self.selected.approval.panel.is_empty() {
                    self.window.output_open = true;
                    self.resize_window(window, cx);
                }
            }
            Update::PermissionResponded {
                session,
                request_id,
                result,
            } => {
                if self.selected.id == Some(session) {
                    self.selected
                        .approval
                        .panel
                        .responded(&request_id, result.map_err(|error| error.presentation()));
                }
            }
            Update::PermissionUnavailable { epoch, message } if epoch == self.selected.epoch => {
                self.selected.approval.panel.clear();
                self.selected.approval.error = Some(message.presentation());
                if let Some(session) = self.selected.id
                    && let Some(outbox) = self.sessions.outboxes.get_mut(&session)
                {
                    outbox.pause();
                }
            }
            Update::HistoryWindow { .. }
            | Update::OlderHistory { .. }
            | Update::HistoryOverview { .. }
            | Update::HistoryLocation { .. }
            | Update::HistoryMeta { .. }
            | Update::Event { .. }
            | Update::Stream { .. }
            | Update::Permissions { .. }
            | Update::PermissionUnavailable { .. } => {}
            Update::Elicitations { .. } | Update::ElicitationUnavailable { .. } => {}
        }
        self.pause_archived();
        self.finish_workspace_removals(window, cx);
        for (id, outbox) in &mut self.sessions.outboxes {
            if self.sessions.runtimes.failure(*id).is_some() {
                outbox.pause();
            }
        }
        self.flush_queue(cx);
    }
}
