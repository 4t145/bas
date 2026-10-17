//! Shared readiness, account and conversation actions for both Agent pages.

use super::*;
use gpui_kit::{SharedString, assets::IconName, px};

impl Agents {
    pub(super) fn check_button(
        &self,
        id: String,
        spec: &AgentSpec,
        cx: &mut Context<Self>,
    ) -> Button {
        let status = self.status(spec);
        let busy = status.is_some_and(Status::busy);
        let icon = match status {
            Some(Status::Connected(_)) => IconName::Check,
            Some(Status::AuthenticationRequired { .. }) => IconName::LogIn,
            Some(Status::Failed(_) | Status::Missing(_)) => IconName::CircleAlert,
            _ => IconName::RefreshCw,
        };
        let hint = status.map_or(crate::i18n::tr!("agent-check"), status::check_label);
        let result_label = status.map_or(crate::i18n::tr!("agent-unchecked"), status::label);
        let group = SharedString::from(format!("{id}-hover"));
        let expected = spec.clone();
        gds_uim::el! {
            Button($id.clone()) {
                ghost; xsmall; w: $px(crate::ui::COMPACT_CONTROL); h: $px(crate::ui::COMPACT_CONTROL);
                p_0; flex_shrink_0; group: $group.clone();
                accessibility_label: hint; tooltip: $format!("{hint}\n{}", self.check_detail(spec));
                loading: busy;
                disabled: $(busy || self.logging_out(spec) || self.dirty(cx) || !self.registry.editable());
                on_click: $cx.listener(move |this, _, _, cx| this.start_check(expected.clone(), true, cx));
            } [
                #div { relative; size: $px(crate::ui::SMALL_ICON); } [
                    #div {
                        id: $format!("{id}-result"); test_support;
                        role: gpui_kit::Role::Label; aria_label: result_label;
                        group_hover: $group.clone(), $gds_uim::m! { invisible; };
                    } [Icon(icon) { size: $px(crate::ui::SMALL_ICON); }],
                    #div {
                        id: $format!("{id}-retry"); test_support;
                        absolute; inset_0; invisible;
                        group_hover: group, $gds_uim::m! { visible; };
                    } [Icon(IconName::RefreshCw) { size: $px(crate::ui::SMALL_ICON); }],
                ],
            ]
        }
    }

    pub(super) fn account_actions(
        &self,
        spec: &AgentSpec,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = spec.id.to_string();
        let can_chat =
            spec.kind == Kind::Embedded || self.status(spec).is_some_and(Status::connected);
        gds_uim::el! { #div { flex; items_center; gap_1; } [
            ..$bool::then(spec.kind == Kind::Acp, || self.logout_button(spec, cx)),
            ..$can_chat.then(|| gds_uim::el! {
                #item::action($format!("agents-registry-start-chat-{id}"), IconName::MessageCircle, $crate::i18n::tr!("agent-start-chat")) {
                    disabled: $(!self.registry.editable() || self.dirty(cx) || self.logging_out(spec));
                    on_click: $cx.listener(move |this, _, _, cx| this.begin_chat(id.clone(), cx));
                }
            }),
        ] }.into_any_element()
    }
}
