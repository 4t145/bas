use super::*;

pub(super) struct State {
    spec: AgentSpec,
    busy: bool,
    message: String,
    _task: Task<()>,
}

impl Agents {
    pub(super) fn logging_out(&self, spec: &AgentSpec) -> bool {
        self.logouts
            .get(&spec.id)
            .is_some_and(|state| state.spec == *spec && state.busy)
    }

    pub(super) fn logout_message(&self, spec: &AgentSpec) -> Option<String> {
        self.logouts
            .get(&spec.id)
            .filter(|state| state.spec == *spec)
            .map(|state| state.message.clone())
    }

    fn can_logout(&self, spec: &AgentSpec, cx: &App) -> bool {
        spec.kind == Kind::Acp
            && self.registry.verified_local()
            && !self.dirty(cx)
            && !self.logging_out(spec)
            && !self.status(spec).is_some_and(Status::busy)
            && !self.authentications.contains_key(&spec.id)
            && self.display_catalog().iter().any(|current| current == spec)
    }

    fn request_logout(&mut self, spec: AgentSpec, cx: &mut Context<Self>) {
        if !self.can_logout(&spec, cx) {
            return;
        }
        let request = crate::application::Confirmation {
            title: crate::i18n::tr!("agent-logout").into(),
            message: crate::i18n::tr!("agent-logout-confirm", agent = spec.id.to_string()),
            confirm_label: crate::i18n::tr!("agent-logout").into(),
        };
        let owner = cx.entity().downgrade();
        if let Err(error) = crate::application::open_confirmation(
            request,
            move |cx| {
                let _ = owner.update(cx, |this, cx| this.logout(spec, cx));
            },
            cx,
        ) {
            self.message = error.to_string();
            cx.notify();
        }
    }

    fn logout(&mut self, spec: AgentSpec, cx: &mut Context<Self>) {
        if !self.can_logout(&spec, cx) {
            return;
        }
        let Some(service) = cx.try_global::<catalog::Service>().cloned() else {
            return;
        };
        let request = service.logout(spec.clone(), self.store.path().to_owned());
        let expected = spec.clone();
        let task = cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                let Some(state) = page
                    .logouts
                    .get_mut(&expected.id)
                    .filter(|state| state.spec == expected)
                else {
                    return;
                };
                state.busy = false;
                state.message = match result {
                    Ok(()) => crate::i18n::tr!("agent-logout-complete").into(),
                    Err(error) => crate::i18n::tr!("agent-logout-failed", error = error),
                };
                // A timeout may already have changed credentials. Invalidate old
                // readiness instead of probing session/new or silently retrying.
                if page
                    .checks
                    .get(&expected.id)
                    .is_some_and(|check| check.spec == expected)
                {
                    page.checks.remove(&expected.id);
                    page.authentications.remove(&expected.id);
                    page.authentication_owners.remove(&expected.id);
                }
                page.notify_agent_change(None, cx);
                cx.notify();
            });
        });
        self.logouts.insert(
            spec.id.clone(),
            State {
                spec,
                busy: true,
                message: crate::i18n::tr!("agent-logout-pending").into(),
                _task: task,
            },
        );
        cx.notify();
    }

    pub(super) fn logout_button(
        &self,
        spec: &AgentSpec,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let expected = spec.clone();
        gds_uim::el! {
            #item::action($format!("agent-logout-{}", spec.id), $gpui_kit::assets::IconName::LogOut, $crate::i18n::tr!("agent-logout")) {
                tooltip: $crate::i18n::tr!("agent-logout-help");
                loading: $self.logging_out(spec);
                disabled: $!self.can_logout(spec, cx);
                on_click: $cx.listener(move |this, _, _, cx| this.request_logout(expected.clone(), cx));
            }
        }.into_any_element()
    }
}
