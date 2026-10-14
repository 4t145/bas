use super::super::*;
use crate::elicitation;

impl Workspace {
    pub(in crate::workspace) fn sync_elicitations(
        &mut self,
        pending: Vec<connection::PendingElicitation>,
        completed: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.selected.id else {
            return;
        };
        let epoch = self.selected.epoch;
        let ids: std::collections::HashSet<_> = pending
            .iter()
            .map(|pending| pending.request_id.as_str())
            .collect();
        self.selected
            .elicitations
            .retain(|id, _| ids.contains(id.as_str()) && !completed.contains(id));
        for pending in pending {
            if completed.contains(&pending.request_id) {
                continue;
            }
            if let Some(panel) = self.selected.elicitations.get(&pending.request_id) {
                if pending.answered {
                    panel.entity.update(cx, |panel, cx| {
                        panel.submitting = true;
                        cx.notify();
                    });
                }
                continue;
            }
            let request = match pending.request {
                Ok(request) => request,
                Err(error) => {
                    self.selected.error = Some(error.presentation());
                    continue;
                }
            };
            let id = pending.request_id;
            let panel = cx.new(|cx| {
                let mut panel = elicitation::Panel::new(id.clone(), request, window, cx);
                panel.submitting = pending.answered;
                panel
            });
            let answered = cx.subscribe(
                &panel,
                move |this, _, answer: &elicitation::Answered, cx| {
                    this.on_session_elicitation_answered(session, epoch, answer, cx);
                },
            );
            let changed = cx.observe(&panel, Self::on_session_elicitation_changed);
            self.selected.elicitations.insert(
                id,
                state::ObservedPanel::new(panel, vec![answered, changed]),
            );
            self.window.output_open = true;
            if self.window.compact {
                self.resize_window(window, cx);
            }
        }
        cx.notify();
    }

    fn on_session_elicitation_answered(
        &mut self,
        session: SessionId,
        epoch: u64,
        answer: &elicitation::Answered,
        cx: &mut Context<Self>,
    ) {
        if self.selected.id == Some(session)
            && self.selected.epoch == epoch
            && self.selected.elicitations.contains_key(&answer.request_id)
        {
            self.command(
                Command::RespondElicitation {
                    session,
                    request_id: answer.request_id.clone(),
                    response: answer.response.clone(),
                },
                cx,
            );
        }
        cx.notify();
    }

    fn on_session_elicitation_changed(
        &mut self,
        _: Entity<elicitation::Panel>,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }
}
