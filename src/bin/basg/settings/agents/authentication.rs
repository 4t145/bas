//! The settings page hosts the same Agent-owned authentication panel as threads.
use super::Agents;
use crate::agents::Status;
use crate::ui::Button;
use bas::config::AgentSpec;
use gpui_kit::{Context, IntoElement, ParentElement, TestSupportExt, Window, div, prelude::*};

pub(super) struct Owner {
    window: gpui_kit::AnyWindowHandle,
    initial: Status,
}

impl Agents {
    pub(super) fn ensure_authentication_panels(
        &mut self,
        page: super::Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Interactive GPUI inputs belong to one native window. If that window
        // closes, cancel its flow and offer the original methods in this one.
        self.authentication_owners.retain(|id, owner| {
            if !self.authentications.contains_key(id) {
                return false;
            }
            if cx.windows().contains(&owner.window) {
                return true;
            }
            self.authentications.remove(id);
            if let Some(check) = self.checks.get_mut(id) {
                check.status = owner.initial.clone();
            }
            false
        });
        let pending: Vec<_> = self
            .checks
            .values()
            .filter(|check| matches!(check.status, Status::AuthenticationRequired { .. }))
            .filter(|check| {
                let added_here = self
                    .saved
                    .as_ref()
                    .is_some_and(|saved| saved == &check.spec)
                    && super::window::current(cx)
                        .is_some_and(|handle| cx.windows().contains(&handle.into()));
                match page {
                    super::Page::Onboarding => added_here,
                    super::Page::Configuration => !added_here,
                }
            })
            .map(|check| (check.spec.clone(), check.status.clone()))
            .collect();
        for (spec, status) in pending {
            if self.authentications.contains_key(&spec.id) {
                continue;
            }
            let panel = cx.new(|_| {
                crate::authentication::Authentication::new(
                    spec.clone(),
                    status.clone(),
                    self.store.path().to_owned(),
                    window.window_handle(),
                )
            });
            let expected = spec.clone();
            self._subscriptions.push(cx.subscribe(
                &panel,
                move |page, _, event: &crate::authentication::Completed, cx| {
                    page.on_authentication_completed(&expected, event, cx);
                },
            ));
            self._subscriptions
                .push(cx.observe(&panel, |_, _, cx| cx.notify()));
            self.authentication_owners.insert(
                spec.id.clone(),
                Owner {
                    window: window.window_handle(),
                    initial: status,
                },
            );
            self.authentications.insert(spec.id.clone(), panel);
        }
        self.authentications
            .retain(|id, _| self.checks.contains_key(id));
    }

    fn on_authentication_completed(
        &mut self,
        expected: &AgentSpec,
        event: &crate::authentication::Completed,
        cx: &mut Context<Self>,
    ) {
        self.apply_check(expected, event.0.clone());
        if event.0.connected() && self.status(expected).is_some_and(Status::connected) {
            self.authentications.remove(&expected.id);
            self.message = crate::i18n::tr!("auth-complete").into();
            self.notify_agent_change(Some(expected.id.to_string()), cx);
        }
        cx.notify();
    }

    pub(super) fn authentication_content(
        &self,
        spec: &AgentSpec,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(panel) = self.authentications.get(&spec.id) else {
            return gds_uim::el! { #div {} }.into_any_element();
        };
        if let Some(owner) = self.authentication_owners.get(&spec.id)
            && self.rendering_window != Some(owner.window)
        {
            let target = owner.window;
            return gds_uim::el! {
                Button($format!("agents-authentication-open-{index}")) {
                    label: $crate::i18n::tr!("agent-auth-other-window");
                    on_click: $cx.listener(move |_, _, _, cx| {
                        let _ = target.update(cx, |_, window, _| window.activate_window());
                    });
                }
            }
            .into_any_element();
        }
        let editable = !self.dirty(cx) && self.registry.editable();
        gds_uim::el! {
            #div {
                id: $format!("agents-authentication-{index}");
                test_support; flex; flex_col; gap_2;
            } [
                ..$bool::then(!editable, || crate::i18n::tr!("auth-finish-edit")),
                ..$editable.then(|| panel.clone()),
            ]
        }
        .into_any_element()
    }
}
