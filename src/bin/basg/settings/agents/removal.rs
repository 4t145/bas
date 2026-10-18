use super::*;
use gpui_kit::{assets::IconName, component::button::ButtonCustomVariant};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

impl Agents {
    fn can_remove(&self, registration: &AgentRegistration, cx: &App) -> bool {
        self.registry.editable()
            && !self.dirty(cx)
            && !self.operations.values().any(|operation| operation.busy)
            && self
                .document
                .as_ref()
                .is_some_and(|document| document.settings.daemon.agents.contains(registration))
            && registration
                .as_custom()
                .is_none_or(|spec| spec.kind != Kind::Embedded)
            && (registration.managed().is_none() || self.managed_allowed(cx))
    }

    pub(super) fn remove_button(&self, id: &str, cx: &mut Context<Self>) -> Button {
        let registration = self
            .document
            .as_ref()
            .and_then(|document| {
                document
                    .settings
                    .daemon
                    .agents
                    .iter()
                    .find(|r| r.id().as_str() == id)
            })
            .cloned();
        let enabled = registration
            .as_ref()
            .is_some_and(|registration| self.can_remove(registration, cx));
        let style = ButtonCustomVariant::new(cx)
            .foreground(cx.theme().danger)
            .hover(cx.theme().danger.opacity(0.12))
            .active(cx.theme().danger.opacity(0.2));
        item::action(
            format!("agents-remove-{id}"),
            IconName::Trash,
            crate::i18n::tr!("agent-mark-remove"),
        )
        .custom(style)
        .disabled(!enabled)
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(registration) = &registration {
                this.request_remove(registration.clone(), cx);
            }
        }))
    }

    fn request_remove(&mut self, registration: AgentRegistration, cx: &mut Context<Self>) {
        if !self.can_remove(&registration, cx) {
            return;
        }
        let request = crate::application::Confirmation {
            title: crate::i18n::tr!("agent-mark-remove").into(),
            message: crate::i18n::tr!("agent-remove-dialog", agent = registration.id().to_string()),
            confirm_label: crate::i18n::tr!("agent-mark-remove").into(),
            destructive: true,
        };
        let owner = cx.entity().downgrade();
        if let Err(error) = crate::application::open_confirmation(
            request,
            move |cx| {
                let _ = owner.update(cx, |this, cx| this.remove_registration(registration, cx));
            },
            cx,
        ) {
            self.message = error.to_string();
            cx.notify();
        }
    }

    fn remove_registration(&mut self, registration: AgentRegistration, cx: &mut Context<Self>) {
        if !self.can_remove(&registration, cx) {
            return;
        }
        // Confirm the saved identity again; never remove a replacement with the same ID.
        match self.store.load() {
            Ok(document) if document.settings.daemon.agents.contains(&registration) => {}
            Ok(_) => {
                self.message = DraftError::Changed.to_string();
                cx.notify();
                return;
            }
            Err(error) => {
                self.message = error.to_string();
                cx.notify();
                return;
            }
        }
        self.saved = None;
        if let Some(spec) = registration.as_custom() {
            let previous = std::mem::replace(
                &mut self.editor,
                Editor::Edit {
                    baseline: spec.clone(),
                    intent: EditIntent::Update,
                },
            );
            self.editor.mark_delete();
            self.save(cx);
            if self.editor.deleting() {
                self.editor = previous;
            } else {
                self.message = crate::i18n::tr!("agent-removed").into();
                self.refresh_registry(false, cx);
            }
        } else {
            self.remove_managed(registration.id().to_string(), cx);
        }
        cx.notify();
    }
}
