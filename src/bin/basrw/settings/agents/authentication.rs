//! The settings page hosts the same Agent-owned authentication panel as threads.
use super::{Agents, check};
use basr::config::AgentSpec;
use gpui_kit::{Context, IntoElement, ParentElement, TestSupportExt, Window, div, prelude::*};

impl Agents {
    pub(super) fn ensure_authentication_panels(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending: Vec<_> = self
            .checks
            .values()
            .filter(|check| matches!(check.status, check::Status::AuthenticationRequired { .. }))
            .map(|check| (check.spec.clone(), check.status.clone()))
            .collect();
        for (spec, status) in pending {
            if self.authentications.contains_key(&spec.id) {
                continue;
            }
            let panel = cx.new(|_| {
                crate::authentication::Authentication::new(
                    spec.clone(),
                    status,
                    self.store.path().to_owned(),
                    window.window_handle(),
                )
            });
            let expected = spec.clone();
            self._subscriptions.push(cx.subscribe(
                &panel,
                move |page, _, event: &crate::authentication::Completed, cx| {
                    page.apply_check(&expected, event.0.clone());
                    if event.0.connected()
                        && page.status(&expected).is_some_and(check::Status::connected)
                    {
                        page.authentications.remove(&expected.id);
                        page.message = crate::i18n::tr!("auth-complete").into();
                        page.notify_agent_change(Some(expected.id.to_string()), cx);
                    }
                    cx.notify();
                },
            ));
            self._subscriptions
                .push(cx.observe(&panel, |_, _, cx| cx.notify()));
            self.authentications.insert(spec.id.clone(), panel);
        }
        self.authentications
            .retain(|id, _| self.checks.contains_key(id));
    }

    pub(super) fn authentication_content(
        &self,
        spec: &AgentSpec,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(panel) = self.authentications.get(&spec.id) else {
            return div().into_any_element();
        };
        div()
            .id(format!("agents-authentication-{index}"))
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .when(self.dirty(cx) || !self.local_editable, |row| {
                row.child(crate::i18n::tr!("auth-finish-edit"))
            })
            .when(!self.dirty(cx) && self.local_editable, |row| {
                row.child(panel.clone())
            })
            .into_any_element()
    }
}
