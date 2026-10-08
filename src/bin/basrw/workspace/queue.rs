use super::*;

const QUEUE_PANEL_MAX_HEIGHT: f32 = 180.;
const QUEUE_PREVIEW_CHARACTERS: usize = 240;

impl Workspace {
    pub(super) fn sending(&self, session: SessionId) -> bool {
        self.outboxes
            .get(&session)
            .is_some_and(outbox::SessionOutbox::sending)
    }

    pub(super) fn enqueue_draft(
        &mut self,
        session: SessionId,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.archive_blocked(session) {
            return;
        }
        let restored = self
            .outboxes
            .entry(session)
            .or_default()
            .enqueue(Draft {
                text,
                attachments: self.attachments.clone(),
            })
            .unwrap_or_default();
        self.attachments = restored.attachments;
        self.composer
            .update(cx, |input, cx| input.set_value(restored.text, window, cx));
        self.save_draft(cx);
        self.error = None;
        self.list.set_follow_mode(FollowMode::Tail);
        self.flush_queue(cx);
        cx.notify();
    }

    pub(super) fn flush_queue(&mut self, cx: &mut Context<Self>) {
        if !self.connected {
            return;
        }
        let sessions = self.sessions.clone();
        for session in sessions {
            if self.archive_blocked(session.id) {
                continue;
            }
            let allowed =
                !self.configuring.contains(&session.id) && !self.cancelling.contains(&session.id);
            let action = self
                .outboxes
                .get_mut(&session.id)
                .and_then(|outbox| outbox.advance(&session, allowed));
            if let Some(outbox::Action::Send(draft)) = action {
                self.command(Command::Prompt {
                    session: session.id,
                    text: draft.text,
                    attachments: draft.attachments,
                    capabilities: session
                        .initialization
                        .as_ref()
                        .map(|init| init.agent_capabilities.prompt_capabilities.clone())
                        .unwrap_or_default(),
                    title: None,
                });
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
            .active
            .and_then(|id| self.outboxes.get_mut(&id))
            .and_then(outbox::SessionOutbox::cancel_edit)
        else {
            return false;
        };
        self.attachments = draft.attachments;
        self.composer
            .update(cx, |input, cx| input.set_value(draft.text, window, cx));
        self.save_draft(cx);
        cx.notify();
        true
    }

    fn edit_queue_draft(
        &mut self,
        session: SessionId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let composition = Draft {
            text: self.composer.read(cx).value().to_string(),
            attachments: self.attachments.clone(),
        };
        let Some(draft) = self
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
        self.attachments = draft.attachments;
        self.composer.update(cx, |input, cx| {
            input.set_value(draft.text, window, cx);
            input.focus(window, cx);
        });
        self.save_draft(cx);
        if self.compact {
            self.resize_window(window, cx);
        }
        cx.notify();
    }

    fn withdraw_queue_draft(
        &mut self,
        session: SessionId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(withdrawal) = self
            .outboxes
            .get_mut(&session)
            .and_then(|outbox| outbox.withdraw(index))
        else {
            return;
        };
        if let Some(composition) = withdrawal.composition {
            self.restore_queue_composition(composition, window, cx);
        }
        cx.notify();
    }

    fn queue_send_ready(&self, session: SessionId) -> bool {
        self.active == Some(session)
            && self.approval_error.is_none()
            && self.connection_status() == status::ConnectionStatus::Connected
            && self.stream == StreamState::Live
            && !self.archive_blocked(session)
            && !self.sending(session)
            && !self.cancelling.contains(&session)
            && !self.configuring.contains(&session)
            && !self.loading_attachments.contains(&session)
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
        let Some(session) = self.active else {
            return;
        };
        let interrupt = self.working();
        let composition = self.outboxes.entry(session).or_default().enqueue_immediate(
            Draft {
                text: self.composer.read(cx).value().to_string(),
                attachments: self.attachments.clone(),
            },
            interrupt,
        );
        self.restore_queue_composition(composition.unwrap_or_default(), window, cx);
        self.dispatch_immediate_queue(session, interrupt, cx);
    }

    fn dispatch_immediate_queue(
        &mut self,
        session: SessionId,
        interrupt: bool,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        self.commands_dismissed = true;
        self.list.set_follow_mode(FollowMode::Tail);
        if interrupt {
            self.cancelling.insert(session);
            self.command(Command::Cancel(session));
        } else {
            self.flush_queue(cx);
        }
        cx.notify();
    }

    pub(super) fn complete_queue_interrupt(&mut self, session: SessionId, succeeded: bool) {
        if let Some(outbox) = self.outboxes.get_mut(&session) {
            outbox.complete_interrupt(succeeded);
        }
    }

    pub(super) fn immediate_send_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("send-immediately")
            .ghost()
            .xsmall()
            .icon(IconName::ArrowUp)
            .tooltip("立即发送 · 打断当前任务后发送此消息")
            .disabled(!self.can_send(cx))
            .on_click(cx.listener(|this, _, window, cx| this.send_immediately(window, cx)))
    }

    pub(super) fn render_queue_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.active?;
        let outbox = self.outboxes.get(&session)?;
        if outbox.len() == 0 {
            return None;
        }
        let editing = outbox.editing();
        let busy = self.sending(session) || self.cancelling.contains(&session);
        let send_ready = self.queue_send_ready(session) && editing.is_none();
        Some(
            div()
                .id("pending-messages-panel")
                .test_support()
                .flex_shrink_0()
                .px_3()
                .pb_2()
                .child(
                    chat::content_panel()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().muted.opacity(0.35))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .py_1()
                                .child(caption(
                                    format!(
                                        "待发送 · {} 条{}",
                                        outbox.len(),
                                        if outbox.paused() { " · 已暂停" } else { "" }
                                    ),
                                    cx,
                                ))
                                .child(div().flex_1())
                                .when(outbox.paused() && editing.is_none(), |header| {
                                    header.child(
                                        Button::new("resume-pending-messages")
                                            .ghost()
                                            .xsmall()
                                            .label("继续发送")
                                            .disabled(!send_ready)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.queue_send_ready(session) {
                                                    return;
                                                }
                                                if let Some(outbox) =
                                                    this.outboxes.get_mut(&session)
                                                {
                                                    outbox.resume();
                                                }
                                                this.error = None;
                                                this.flush_queue(cx);
                                            })),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .id("pending-messages-list")
                                .max_h(px(QUEUE_PANEL_MAX_HEIGHT))
                                .overflow_y_scroll()
                                .children(outbox.iter().enumerate().map(|(index, draft)| {
                                    div()
                                        .id(SharedString::from(format!("pending-message-{index}")))
                                        .test_support()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_3()
                                        .py_1()
                                        .when(editing == Some(index), |row| {
                                            row.bg(cx.theme().accent)
                                        })
                                        .child(
                                            div().flex_1().min_w_0().text_sm().truncate().child(
                                                queue_summary(draft, QUEUE_PREVIEW_CHARACTERS),
                                            ),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "edit-pending-message-{index}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .label(if editing == Some(index) {
                                                "编辑中"
                                            } else {
                                                "修改"
                                            })
                                            .disabled(busy || editing.is_some())
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.edit_queue_draft(session, index, window, cx)
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "withdraw-pending-message-{index}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .label("撤回")
                                            .disabled(busy)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.withdraw_queue_draft(
                                                    session, index, window, cx,
                                                )
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "send-pending-message-{index}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .label(if self.working() {
                                                "打断发送"
                                            } else {
                                                "发送"
                                            })
                                            .disabled(!send_ready)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.send_queue_draft_immediately(
                                                    session, index, cx,
                                                )
                                            })),
                                        )
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.active?;
        let outbox = self.outboxes.get(&session)?;
        let draft = outbox.front()?;
        let summary = queue_summary(draft, QUEUE_PREVIEW_CHARACTERS);
        let hint = format!(
            "{} 条待发送消息{} · 点击取回编辑 · Escape 取消编辑\n{summary}",
            outbox.len(),
            if outbox.paused() {
                "（已暂停）"
            } else {
                ""
            },
        );
        Some(
            crate::tooltip::bounded(
                "pending-messages-hint",
                Button::new("pending-messages")
                    .disabled(self.sending(session) || outbox.editing().is_some())
                    .ghost()
                    .xsmall()
                    .icon(IconName::Clock)
                    .label(outbox.len().to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_queue_draft(session, 0, window, cx);
                    })),
                hint,
            )
            .into_any_element(),
        )
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
