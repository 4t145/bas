use super::*;

impl Workspace {
    pub(super) fn enqueue_draft(
        &mut self,
        session: SessionId,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.queued_paused.remove(&session);
        self.queued.entry(session).or_default().push_back(Draft {
            text,
            attachments: self.attachments.clone(),
        });
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.attachments.clear();
        self.drafts.remove(&session);
        self.error = None;
        self.flush_queue(cx);
        cx.notify();
    }

    pub(super) fn flush_queue(&mut self, cx: &mut Context<Self>) {
        if !self.connected {
            return;
        }
        // A send receipt can precede its TurnStarted event. Wait for newer session
        // state before treating the old Idle snapshot as permission to send again.
        self.awaiting_turn.retain(|id, sent_at| {
            let meta = self
                .conversation
                .as_ref()
                .filter(|c| c.session.id == *id)
                .map(|c| &c.session)
                .or_else(|| self.sessions.iter().find(|meta| meta.id == *id));
            meta.is_none_or(|meta| meta.updated_at <= *sent_at)
        });
        let ready: Vec<_> = self
            .queued
            .iter()
            .filter(|(id, queue)| {
                !queue.is_empty()
                    && !self.queued_paused.contains(id)
                    && !self.sending.contains(id)
                    && !self.awaiting_turn.contains_key(id)
                    && !self.configuring.contains(id)
                    && !self.cancelling.contains(id)
                    && !self.queued_inflight.contains_key(id)
            })
            .filter_map(|(id, _)| {
                let meta = self
                    .conversation
                    .as_ref()
                    .filter(|c| c.session.id == *id)
                    .map(|c| &c.session)
                    .or_else(|| self.sessions.iter().find(|meta| meta.id == *id))?;
                (meta.status == SessionStatus::Idle).then(|| meta.clone())
            })
            .collect();
        for session in ready {
            let Some(draft) = self
                .queued
                .get_mut(&session.id)
                .and_then(|queue| queue.pop_front())
            else {
                continue;
            };
            self.sending.insert(session.id);
            self.awaiting_turn.insert(session.id, session.updated_at);
            self.queued_inflight.insert(session.id, draft.clone());
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
        cx.notify();
    }

    pub(super) fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.active?;
        let queue = self
            .queued
            .get(&session)
            .filter(|queue| !queue.is_empty())?;
        let draft = queue.front()?;
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
            "{} 条待发送消息{} · 点击取回编辑\n{summary}",
            queue.len(),
            if self.queued_paused.contains(&session) {
                "（已暂停）"
            } else {
                ""
            }
        );
        Some(
            Button::new("pending-messages")
                .ghost()
                .xsmall()
                .icon(IconName::Clock)
                .label(queue.len().to_string())
                .tooltip(hint)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let Some(draft) = this
                        .queued
                        .get_mut(&session)
                        .and_then(|queue| queue.pop_front())
                    else {
                        return;
                    };
                    this.queued_paused.insert(session);
                    // Keep an existing composition as the next queued draft instead of overwriting it.
                    let current = this.composer.read(cx).value().to_string();
                    if !current.trim().is_empty() {
                        this.queued.entry(session).or_default().push_front(Draft {
                            text: current,
                            attachments: this.attachments.clone(),
                        });
                    }
                    this.attachments = draft.attachments;
                    this.composer.update(cx, |input, cx| {
                        input.set_value(draft.text, window, cx);
                        input.focus(window, cx);
                    });
                    cx.notify();
                }))
                .into_any_element(),
        )
    }
}
