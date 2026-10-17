use super::*;
use gds_uim::el;

const QUEUE_PANEL_MAX_HEIGHT: f32 = 180.;
const QUEUE_PREVIEW_CHARACTERS: usize = 240;

impl Workspace {
    pub(super) fn sending(&self, session: SessionId) -> bool {
        self.sessions
            .outboxes
            .get(&session)
            .is_some_and(outbox::SessionOutbox::sending)
    }

    pub(super) fn composition_locked(&self, session: SessionId) -> bool {
        self.sessions
            .outboxes
            .get(&session)
            .is_some_and(outbox::SessionOutbox::composition_locked)
    }

    pub(super) fn enqueue_draft(
        &mut self,
        session: SessionId,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.archive_blocked(session) || !self.sessions.healthy(session) {
            return;
        }
        let restored = self
            .sessions
            .outboxes
            .entry(session)
            .or_default()
            .enqueue(Draft {
                text,
                attachments: self.composer.attachments.clone(),
            })
            .unwrap_or_default();
        self.composer.replace(restored, window, cx);
        self.save_draft(cx);
        self.selected.error = None;
        self.selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        self.flush_queue(cx);
        cx.notify();
    }

    pub(super) fn flush_queue(&mut self, cx: &mut Context<Self>) {
        if !self.backend.connected {
            return;
        }
        let sessions = self.sessions.entries.clone();
        for session in sessions {
            if self.archive_blocked(session.id) || !self.sessions.healthy(session.id) {
                continue;
            }
            let allowed = !self.sessions.pending.configuring.contains(&session.id)
                && !self.sessions.pending.cancelling.contains(&session.id);
            let action = self
                .sessions
                .outboxes
                .get_mut(&session.id)
                .and_then(|outbox| outbox.advance(&session, allowed));
            if let Some(send) = action {
                let capabilities = session
                    .initialization
                    .as_ref()
                    .map(|init| init.agent_capabilities.prompt_capabilities.clone())
                    .unwrap_or_default();
                let submission = crate::submission::prepare(
                    session.id,
                    send.turn,
                    &send.draft.text,
                    &send.draft.attachments,
                    &capabilities,
                );
                self.command(
                    Command::Prompt {
                        session: session.id,
                        submission,
                        title: None,
                    },
                    cx,
                );
            }
        }
        cx.notify();
    }

    pub(super) fn cancel_queue_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(draft) = self
            .selected
            .id
            .and_then(|id| self.sessions.outboxes.get_mut(&id))
            .and_then(outbox::SessionOutbox::cancel_edit)
        else {
            return false;
        };
        self.composer.switch_target(draft, window, cx);
        self.save_draft(cx);
        cx.notify();
        true
    }

    pub(super) fn edit_queue_draft(
        &mut self,
        session: SessionId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id != Some(session) {
            return;
        }
        let composition = self.composer.draft(cx);
        let Some(draft) = self
            .sessions
            .outboxes
            .get_mut(&session)
            .and_then(|outbox| outbox.begin_edit_at(index, composition))
        else {
            return;
        };
        self.restore_queue_composition(draft, window, cx);
    }

    fn restore_queue_composition(
        &mut self,
        draft: Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.switch_target(draft, window, cx);
        self.focus_input(window, cx);
        self.save_draft(cx);
        if self.window.compact {
            self.resize_window(window, cx);
        }
        cx.notify();
    }

    pub(super) fn withdraw_queue_draft(
        &mut self,
        session: SessionId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id != Some(session) {
            return;
        }
        let composition = self.composer.draft(cx);
        let Some(withdrawal) = self
            .sessions
            .outboxes
            .get_mut(&session)
            .and_then(|outbox| outbox.withdraw(index, composition))
        else {
            return;
        };
        if let Some(draft) = withdrawal.retrieved {
            self.restore_queue_composition(draft, window, cx);
        }
        cx.notify();
    }

    fn queue_send_ready(&self, session: SessionId) -> bool {
        self.selected.id == Some(session)
            && self.sessions.healthy(session)
            && self.selected.approval.error.is_none()
            && self.connection_status() == status::ConnectionStatus::Connected
            && self.selected.stream == StreamState::Live
            && !self.archive_blocked(session)
            && !self.sending(session)
            && !self.sessions.pending.cancelling.contains(&session)
            && !self.sessions.pending.configuring.contains(&session)
            && !self.sessions.pending.loading_attachments.contains(&session)
            && self.current().is_some_and(|meta| {
                matches!(meta.status, SessionStatus::Idle | SessionStatus::Working)
            })
    }

    fn send_queue_draft_immediately(
        &mut self,
        session: SessionId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.queue_send_ready(session) {
            return;
        }
        let interrupt = self.working();
        if !self
            .sessions
            .outboxes
            .get_mut(&session)
            .is_some_and(|outbox| outbox.prioritize(index, interrupt))
        {
            return;
        }
        self.dispatch_immediate_queue(session, interrupt, cx);
    }

    pub(super) fn send_immediately(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        let Some(session) = self.selected.id else {
            return;
        };
        let interrupt = self.working();
        let composition = self
            .sessions
            .outboxes
            .entry(session)
            .or_default()
            .enqueue_immediate(self.composer.draft(cx), interrupt);
        self.restore_queue_composition(composition.unwrap_or_default(), window, cx);
        self.dispatch_immediate_queue(session, interrupt, cx);
    }

    fn dispatch_immediate_queue(
        &mut self,
        session: SessionId,
        interrupt: bool,
        cx: &mut Context<Self>,
    ) {
        self.selected.error = None;
        self.composer.commands.dismissed = true;
        self.selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Tail);
        if interrupt {
            self.sessions.pending.cancelling.insert(session);
            self.command(Command::Cancel(session), cx);
        } else {
            self.flush_queue(cx);
        }
        cx.notify();
    }

    pub(super) fn complete_queue_interrupt(&mut self, session: SessionId, succeeded: bool) {
        if let Some(outbox) = self.sessions.outboxes.get_mut(&session) {
            outbox.complete_interrupt(succeeded);
        }
    }

    pub(super) fn immediate_send_button(&self, cx: &mut Context<Self>) -> Button {
        el! {
            Button("send-immediately") {
                ghost; xsmall;
                icon: IconName::ArrowUp;
                tooltip: $crate::i18n::tr!("queue-send-now");
                disabled: $!self.can_send(cx);
                on_click: $cx.listener(Self::on_queue_send_immediately);
            }
        }
    }

    fn on_queue_send_immediately(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_immediately(window, cx);
    }

    fn resume_queue(&mut self, session: SessionId, cx: &mut Context<Self>) {
        if !self.queue_send_ready(session) {
            return;
        }
        if let Some(outbox) = self.sessions.outboxes.get_mut(&session) {
            outbox.resume();
        }
        self.selected.error = None;
        self.flush_queue(cx);
    }

    pub(super) fn render_queue_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.selected.id?;
        let outbox = self.sessions.outboxes.get(&session)?;
        if outbox.len() == 0 && outbox.unknown().is_none() {
            return None;
        }
        let editing = outbox.editing();
        let editing_active = outbox.editing_active();
        let busy = self.sending(session) || self.sessions.pending.cancelling.contains(&session);
        let send_ready = self.queue_send_ready(session) && !editing_active;
        let unknown = outbox.unknown().is_some().then(|| {
            el! {
                #div { flex; flex_wrap; items_center; gap_2; px_3; py_1; } [
                    #caption($crate::i18n::tr!("submission-unknown"), cx),
                    Button("query-submission") {
                        ghost; xsmall;
                        label: $crate::i18n::tr!("submission-query");
                        disabled: $outbox.resolving();
                        on_click: $cx.listener(move |this, _, _, cx| {
                            this.resolve_submission(session, false, cx)
                        });
                    },
                    Button("retry-submission") {
                        ghost; xsmall;
                        label: $crate::i18n::tr!("submission-retry");
                        disabled: $outbox.resolving()
                            || self.archive_blocked(session)
                            || !self.sessions.healthy(session);
                        on_click: $cx.listener(move |this, _, _, cx| {
                            this.resolve_submission(session, true, cx)
                        });
                    },
                ]
            }
        });
        let count = if outbox.paused() {
            crate::i18n::tr!("queue-paused-count", count = outbox.len())
        } else {
            crate::i18n::tr!("queue-count", count = outbox.len())
        };
        let resume = (outbox.paused() && !editing_active).then(|| {
            el! {
                Button("resume-pending-messages") {
                    ghost; xsmall;
                    label: $crate::i18n::tr!("queue-resume");
                    disabled: $!send_ready;
                    on_click: $cx.listener(move |this, _, _, cx| this.resume_queue(session, cx));
                }
            }
        });
        let header = el! {
            #div { flex; items_center; gap_2; px_3; py_1; } [
                #caption(count, cx),
                #div { flex_1; },
                ..resume,
            ]
        };
        let rows = outbox.iter().enumerate().map(|(index, draft)| {
            let edit_hint = if editing == Some(index) {
                crate::i18n::tr!("queue-editing")
            } else {
                crate::i18n::tr!("action-edit")
            };
            let edit = crate::ui::icon_button(
                SharedString::from(format!("edit-pending-message-{index}")),
                IconName::Pencil,
            )
            .accessibility_label(edit_hint)
            .tooltip(edit_hint)
            .disabled(busy || editing_active)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.edit_queue_draft(session, index, window, cx)
            }));
            let withdraw_hint = crate::i18n::tr!("queue-withdraw");
            let withdraw = crate::ui::icon_button(
                SharedString::from(format!("withdraw-pending-message-{index}")),
                IconName::ArrowLeft,
            )
            .accessibility_label(withdraw_hint)
            .tooltip(withdraw_hint)
            .disabled(busy)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.withdraw_queue_draft(session, index, window, cx)
            }));
            let send_hint = if self.working() {
                crate::i18n::tr!("queue-interrupt")
            } else {
                crate::i18n::tr!("action-send")
            };
            let send = crate::ui::icon_button(
                SharedString::from(format!("send-pending-message-{index}")),
                IconName::Send,
            )
            .accessibility_label(send_hint)
            .tooltip(send_hint)
            .disabled(!send_ready)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.send_queue_draft_immediately(session, index, cx)
            }));
            el! {
                #div {
                    id: $SharedString::from(format!("pending-message-{index}"));
                    test_support;
                    flex; items_center; gap_2; px_3; py_1;
                    @$editing == Some(index) => { bg: $cx.theme().accent; };
                } [
                    #div {
                        flex_1; min_w_0;
                        text_size: crate::ui::SMALL;
                        truncate;
                    } [$queue_summary(draft, QUEUE_PREVIEW_CHARACTERS)],
                    $edit,
                    $withdraw,
                    $send,
                ]
            }
        });
        let list = el! {
            #div {
                id: "pending-messages-list";
                max_h: QUEUE_PANEL_MAX_HEIGHT.px;
                overflow_y_scroll;
            } [..rows]
        };
        Some(
            el! {
                #div {
                    id: "pending-messages-panel";
                    test_support;
                    flex_shrink_0;
                    @$self.window.compact => { px_3; };
                    pb_2;
                } [
                    #chat::content_panel {
                        rounded_md; border_1;
                        border_color: $cx.theme().border;
                        bg: $cx.theme().muted.opacity(0.35);
                    } [
                        ..unknown,
                        $header,
                        $list,
                    ],
                ]
            }
            .into_any_element(),
        )
    }

    pub(super) fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.selected.id?;
        let outbox = self.sessions.outboxes.get(&session)?;
        outbox.front()?;
        let hint = crate::i18n::tr!("queue-hint");
        let button = el! {
            Button("pending-messages") {
                disabled: $self.sending(session) || outbox.editing().is_some();
                ghost; xsmall;
                icon: IconName::Clock;
                label: $outbox.len().to_string();
                on_click: $cx.listener(move |this, _, window, cx| {
                    this.edit_queue_draft(session, 0, window, cx);
                });
            }
        };
        Some(crate::ui::tooltip::bounded("pending-messages-hint", button, hint).into_any_element())
    }
}

fn queue_summary(draft: &Draft, limit: usize) -> String {
    let mut text = draft.text.clone();
    for attachment in &draft.attachments {
        text = text.replace(&attachment.mention, &attachment.label);
    }
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}
