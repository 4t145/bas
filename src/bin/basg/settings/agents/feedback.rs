//! A saved configuration and the outcome of starting it are separate results.

use super::*;
use gpui_kit::assets::IconName;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

impl Agents {
    pub(super) fn saved_feedback(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        if self.editor.spec().is_some() {
            return None;
        }
        let spec = self
            .saved
            .as_ref()
            .filter(|saved| self.display_catalog().iter().any(|spec| spec == *saved))?;
        let status = self.status(spec);
        let label = match status {
            Some(Status::Checking | Status::Connecting) => {
                crate::i18n::tr!("agent-saved-connecting", agent = spec.id.to_string())
            }
            Some(Status::Failed(_) | Status::Missing(_)) => {
                crate::i18n::tr!("agent-saved-unavailable", agent = spec.id.to_string())
            }
            Some(Status::AuthenticationRequired { .. } | Status::Authenticating) => {
                crate::i18n::tr!("agent-saved-signin", agent = spec.id.to_string())
            }
            _ => crate::i18n::tr!("agent-saved", agent = spec.id.to_string()),
        };
        let error = match status {
            Some(Status::Failed(error) | Status::Missing(error))
                if !self.authentications.contains_key(&spec.id) =>
            {
                Some(error.clone())
            }
            _ => None,
        };
        let id = spec.id.to_string();
        let manage_id = id.clone();
        let owner = cx.entity().downgrade();
        Some(gds_uim::el! { #div {
            id: "agents-save-feedback"; test_support; flex; flex_col; gap_2;
            p_3; border_1; border_color: $cx.theme().border; rounded_md;
        } [
            #div {
                id: "agents-save-result"; test_support; role: gpui_kit::Role::Status;
                aria_label: $label.clone(); text_size: crate::ui::SMALL;
            } [$label],
            ..$error.map(|error| gds_uim::el! {
                #div { id: "agents-save-error"; test_support; text_color: $cx.theme().danger; text_size: crate::ui::SMALL; } [$error]
            }),
            #div { flex; items_center; gap_2; } [
                Button("agents-saved-chat") {
                    small; icon: IconName::MessageCircle; label: $crate::i18n::tr!("agent-start-chat");
                    disabled: $(!status.is_some_and(Status::connected) || !self.registry.editable() || self.logging_out(spec));
                    on_click: $cx.listener(move |this, _, _, cx| this.begin_chat(id.clone(), cx));
                },
                Button("agents-saved-manage") {
                    small; icon: IconName::Settings; label: $crate::i18n::tr!("agent-manage");
                    on_click: $move |_, _, cx| {
                        let id = manage_id.clone();
                        let owner = owner.clone();
                        cx.defer(move |cx| {
                            if let Err(error) = crate::settings::window::open_agent_management(&id, cx) {
                                let _ = owner.update(cx, |page, cx| { page.message = error.to_string(); cx.notify(); });
                            }
                        });
                    };
                },
            ],
        ] }.into_any_element())
    }
}
