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
            _ => IconName::Activity,
        };
        let hint = status.map_or(crate::i18n::tr!("agent-check"), status::check_label);
        let result_label = status.map_or(crate::i18n::tr!("agent-unchecked"), status::label);
        let group = SharedString::from(format!("{id}-hover"));
        let expected = spec.clone();
        el! {
            (~id).Button::new {
                ghost; xsmall; w: ui::COMPACT_CONTROL.px; h: ui::COMPACT_CONTROL.px;
                p_0; flex_shrink_0; group: ~group;
                accessibility_label: hint; tooltip: format!("{hint}\n{}", self.check_detail(spec));
                loading: busy;
                disabled: $(busy || self.logging_out(spec) || self.dirty(cx) || !self.registry.editable());
                on_click: $cx.listener(move |this, _, _, cx| this.start_check(expected.clone(), true, cx));
            } [
                .div { relative; size: ui::SMALL_ICON.px; } [
                    .div {
                        # format!("{id}-result"); test_support;
                        role: gpui_kit::Role::Label; aria_label: result_label;
                        group_hover: ~group, { invisible; };
                    } [(icon).Icon::new { size: ui::SMALL_ICON.px; }],
                    .div {
                        # format!("{id}-retry"); test_support;
                        absolute; inset_0; invisible;
                        group_hover: group, { visible; };
                    } [(IconName::Activity).Icon::new { size: ui::SMALL_ICON.px; }],
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
        el! { .div { flex; items_center; gap_1; } [
            @if (spec.kind == Kind::Acp)
                $self.logout_button(spec, cx),
            @end,
            @if (can_chat)
                (format!("agents-registry-start-chat-{id}"), IconName::MessageCircle, tr!("agent-start-chat")).item::action {
                    disabled: $(!self.registry.editable() || self.dirty(cx) || self.logging_out(spec));
                    on_click: $cx.listener(move |this, _, _, cx| this.begin_chat(id.clone(), cx));
                },
            @end,
        ] }.into_any_element()
    }
}
