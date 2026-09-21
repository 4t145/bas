use super::*;

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

    pub(super) fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.active?;
        let outbox = self.outboxes.get(&session)?;
        let draft = outbox.front()?;
        let mut summary = draft.text.clone();
        for attachment in &draft.attachments {
            summary = summary.replace(&attachment.mention, &attachment.label);
        }
        let summary = summary
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(120)
            .collect::<String>();
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
            Button::new("pending-messages")
                .disabled(self.sending(session))
                .ghost()
                .xsmall()
                .icon(IconName::Clock)
                .label(outbox.len().to_string())
                .tooltip(hint)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let composition = Draft {
                        text: this.composer.read(cx).value().to_string(),
                        attachments: this.attachments.clone(),
                    };
                    let Some(draft) = this
                        .outboxes
                        .get_mut(&session)
                        .and_then(|outbox| outbox.begin_edit(composition))
                    else {
                        return;
                    };
                    this.attachments = draft.attachments;
                    this.composer.update(cx, |input, cx| {
                        input.set_value(draft.text, window, cx);
                        input.focus(window, cx);
                    });
                    this.save_draft(cx);
                    cx.notify();
                }))
                .into_any_element(),
        )
    }
}
