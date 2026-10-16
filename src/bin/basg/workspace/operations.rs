//! Local admission failures finish the same operation state as remote failures.

use super::*;

impl Workspace {
    pub(super) fn respond_permission(
        &mut self,
        session: SessionId,
        epoch: u64,
        request_id: String,
        outcome: bas::acp::RequestPermissionOutcome,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id != Some(session)
            || self.selected.epoch != epoch
            || !self.selected.approval.panel.submitting(&request_id)
        {
            return;
        }
        self.command(
            Command::RespondPermission {
                session,
                request_id,
                outcome,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn command(&mut self, command: Command, cx: &mut Context<Self>) -> bool {
        let target = match &command {
            Command::Rename { session, .. }
            | Command::Archive { session, .. }
            | Command::Delete(session)
            | Command::CloseSession(session)
            | Command::Prompt { session, .. }
            | Command::Cancel(session)
            | Command::ResolvePrompt { session, .. }
            | Command::RespondPermission { session, .. }
            | Command::RespondElicitation { session, .. }
            | Command::Watch { session, .. } => Some(*session),
            Command::SetConfig(request) => request.session_id.to_string().parse().ok(),
            _ => None,
        };
        if let Some(session) = target
            && matches!(
                command,
                Command::Rename { .. }
                    | Command::Archive { .. }
                    | Command::Delete(_)
                    | Command::Prompt { .. }
                    | Command::Cancel(_)
                    | Command::SetConfig(_)
            )
        {
            self.sessions.errors.remove(&session);
            if self.selected.id == Some(session) {
                self.selected.error = None;
            }
        }
        let removed = target.is_some_and(|id| self.sessions.removed.contains(&id));
        let sent = if removed {
            Err(Box::new(command))
        } else {
            self.backend.connection.send(command)
        };
        let Err(command) = sent else {
            return true;
        };
        let message = if removed {
            crate::i18n::tr!("session-deleted").to_owned()
        } else {
            let message = crate::i18n::tr!("connection-reopen").to_owned();
            self.backend.connection_lost(message.clone().into());
            message
        };
        match *command {
            Command::AgentHistory { .. }
            | Command::ImportHistory { .. }
            | Command::RestartSession(_)
            | Command::DeleteAgentHistory { .. } => {
                self.selected.error = Some(message);
            }
            Command::CloseSession(session) => {
                self.sessions.pending.closing.remove(&session);
                self.session_operation_error(
                    session,
                    crate::i18n::tr!("agent-session-close"),
                    message,
                    cx,
                );
            }
            Command::SetConfig(request) => self.configured(
                request
                    .session_id
                    .to_string()
                    .parse()
                    .expect("GUI session ID"),
                Err(message),
            ),
            Command::Cancel(session) => self.cancelled(session, Err(message)),
            Command::Rename { session, .. } => self.rename_completed(session, Err(message)),
            Command::Archive { session, .. } => {
                self.sessions.pending.archiving.remove(&session);
                self.session_operation_error(
                    session,
                    crate::i18n::tr!("session-archive-update-failed"),
                    message,
                    cx,
                );
            }
            Command::Delete(session) => {
                self.sessions.pending.deleting.remove(&session);
                self.session_operation_error(
                    session,
                    crate::i18n::tr!("session-delete-failed"),
                    crate::i18n::tr!("session-delete-reopen").into(),
                    cx,
                );
            }
            Command::Prompt {
                session,
                submission,
                ..
            } => {
                if let Some(outbox) = self.sessions.outboxes.get_mut(&session)
                    && let Some(outbox::Settlement::Rejected {
                        origin,
                        draft,
                        message,
                    }) = outbox.not_sent(submission.turn, message)
                {
                    if origin == outbox::Origin::Composer {
                        self.sessions.drafts.insert(session, draft);
                    }
                    self.session_error(session, message);
                }
            }
            Command::ResolvePrompt {
                session,
                submission,
                ..
            } => {
                if let Some(outbox) = self.sessions.outboxes.get_mut(&session) {
                    outbox.resolution_not_sent(submission.turn);
                }
                self.session_error(session, message);
            }
            Command::RespondPermission {
                session,
                request_id,
                ..
            } => {
                if self.selected.id == Some(session) {
                    self.selected
                        .approval
                        .panel
                        .responded(&request_id, Err(message));
                }
            }
            Command::RespondElicitation {
                session,
                request_id,
                ..
            } => {
                if self.selected.id == Some(session)
                    && let Some(panel) = self.selected.elicitations.get(&request_id)
                {
                    panel
                        .entity
                        .update(cx, |panel, cx| panel.failed(message, cx));
                }
            }
            Command::Create(request) => {
                if self.selected.id.is_none() && self.selected.epoch == request.epoch {
                    self.selected.creating = false;
                    self.selected.error = Some(message);
                }
            }
            Command::Watch { session, epoch } => {
                if self.selected.id == Some(session) && self.selected.epoch == epoch {
                    self.selected.stream = StreamState::Unavailable(message.into());
                }
            }
            Command::Refresh | Command::Detach | Command::Foreground(_) => {}
        }
        cx.notify();
        false
    }

    pub(super) fn configured(
        &mut self,
        session: SessionId,
        result: Result<bas::acp::SetSessionConfigOptionResponse, String>,
    ) {
        self.sessions.pending.configuring.remove(&session);
        // Successful configuration arrives in the ordered event stream.
        if let Err(error) = result {
            self.session_error(session, error);
        }
    }

    pub(super) fn cancelled(&mut self, session: SessionId, result: Result<(), String>) {
        self.sessions.pending.cancelling.remove(&session);
        self.complete_queue_interrupt(session, result.is_ok());
        if let Err(error) = result {
            self.session_error(session, crate::i18n::tr!("task-stop-error", error = error));
        }
    }

    pub(super) fn session_error(&mut self, session: SessionId, message: String) {
        if !self.sessions.removed.contains(&session) {
            self.sessions.errors.insert(session, message);
        }
    }

    pub(super) fn clear_feedback(&mut self) -> bool {
        let local = self.selected.error.take().is_some();
        let session = self
            .selected
            .id
            .and_then(|id| self.sessions.errors.remove(&id))
            .is_some();
        local || session
    }

    pub(super) fn session_operation_error(
        &mut self,
        session: SessionId,
        title: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.session_error(session, message.clone());
        if let Err(error) = crate::application::open_message(title, &message, cx) {
            tracing::error!(%error, %message, "cannot show operation failure");
        }
        cx.notify();
    }
}
