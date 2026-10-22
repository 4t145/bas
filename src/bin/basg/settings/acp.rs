//! Server-owned policy editing; never substitutes local settings for a remote service.
use crate::agents::catalog::Service;
use bas::{
    config::{AcpLifecycle, AgentRegistration, agent::Kind},
    model::AgentId,
    service::pb,
};
use gpui_kit::{
    App, Context, Entity, FocusHandle, ScrollHandle, Subscription, Task, Window,
    component::input::*, prelude::*,
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
mod view;

pub(super) struct Page {
    service: Option<Service>,
    baseline: Option<pb::AcpConfiguration>,
    policy: Option<AcpLifecycle>,
    agents: Vec<AgentRegistration>,
    idle: Entity<InputState>,
    size: Entity<InputState>,
    default_agent: Option<AgentId>,
    pools: Vec<(AgentId, Entity<InputState>)>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    task: Option<Task<()>>,
    pub message: String,
    subscriptions: Vec<Subscription>,
    pool_subscriptions: Vec<Subscription>,
}

impl Page {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let idle = cx.new(|cx| InputState::new(window, cx));
        let size = cx.new(|cx| InputState::new(window, cx));
        let mut page = Self {
            service: cx.try_global::<Service>().cloned(),
            baseline: None,
            policy: None,
            agents: Vec::new(),
            idle,
            size,
            default_agent: None,
            pools: Vec::new(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            task: None,
            message: String::new(),
            subscriptions: Vec::new(),
            pool_subscriptions: Vec::new(),
        };
        for input in [&page.idle, &page.size] {
            page.subscriptions
                .push(cx.subscribe(input, |_, _, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }));
        }
        page
    }

    pub(super) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        if self.baseline.is_none() {
            self.request(None, window, cx);
        }
    }

    fn request(
        &mut self,
        save: Option<pb::SaveAcpConfigurationRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.task.is_some() {
            return;
        }
        let Some(service) = &self.service else {
            self.message = crate::i18n::tr!("acp-policy-no-service").into();
            cx.notify();
            return;
        };
        let saving = save.is_some();
        let request = service.acp_configuration(save);
        self.message = crate::i18n::tr!("acp-policy-loading").into();
        self.task = Some(cx.spawn_in(window, async move |page, cx| {
            let result = request.await;
            let _ = page.update_in(cx, |page, window, cx| {
                page.task = None;
                match result.and_then(|value| page.accept(value, window, cx)) {
                    Ok(()) => {
                        page.message = if saving {
                            crate::i18n::tr!("acp-policy-saved").into()
                        } else {
                            String::new()
                        }
                    }
                    Err(error) => page.message = error,
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn accept(
        &mut self,
        value: pb::AcpConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let policy: AcpLifecycle =
            serde_json::from_slice(&value.policy_json).map_err(|e| e.to_string())?;
        let agents: Vec<AgentRegistration> =
            serde_json::from_slice(&value.registrations_json).map_err(|e| e.to_string())?;
        bas::config::validate_registrations(&agents).map_err(|e| e.to_string())?;
        policy.validate(&agents).map_err(|e| e.to_string())?;
        self.idle.update(cx, |input, cx| {
            input.set_value(
                policy
                    .idle_timeout_secs
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        self.size.update(cx, |input, cx| {
            input.set_value(policy.pool_size.to_string(), window, cx)
        });
        self.default_agent = policy.default_agent.clone();
        self.pools.clear();
        self.pool_subscriptions.clear();
        for agent in agents
            .iter()
            .filter(|a| a.as_custom().is_none_or(|s| s.kind == Kind::Acp))
        {
            let input = cx.new(|cx| {
                InputState::new(window, cx).default_value(
                    policy
                        .pools
                        .get(agent.id())
                        .map(usize::to_string)
                        .unwrap_or_default(),
                )
            });
            self.pool_subscriptions
                .push(cx.subscribe(&input, |_, _, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }));
            self.pools.push((agent.id().clone(), input));
        }
        self.agents = agents;
        self.policy = Some(policy);
        self.baseline = Some(value);
        Ok(())
    }

    fn candidate(&self, cx: &App) -> Result<AcpLifecycle, String> {
        let idle = self.idle.read(cx).value();
        let idle_timeout_secs = if idle.trim().is_empty() {
            None
        } else {
            Some(
                idle.trim()
                    .parse::<std::num::NonZeroU64>()
                    .map_err(|_| crate::i18n::tr!("acp-policy-invalid-timeout").to_owned())?,
            )
        };
        let number = |value: &str| {
            value
                .trim()
                .parse::<usize>()
                .map_err(|_| crate::i18n::tr!("acp-policy-invalid-size").to_owned())
        };
        let mut pools = std::collections::BTreeMap::new();
        for (id, input) in &self.pools {
            let value = input.read(cx).value();
            if !value.trim().is_empty() {
                pools.insert(id.clone(), number(&value)?);
            }
        }
        let policy = AcpLifecycle {
            idle_timeout_secs,
            default_agent: self.default_agent.clone(),
            pool_size: number(&self.size.read(cx).value())?,
            pools,
        };
        policy.validate(&self.agents).map_err(|e| e.to_string())?;
        Ok(policy)
    }

    fn dirty(&self, cx: &App) -> bool {
        self.policy
            .as_ref()
            .is_some_and(|saved| self.candidate(cx).as_ref() != Ok(saved))
    }

    fn disabled(&self) -> bool {
        self.baseline.is_none() || self.task.is_some()
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled() {
            return;
        }
        let Some(baseline) = &self.baseline else {
            return;
        };
        match self.candidate(cx) {
            Ok(policy) => self.request(
                Some(pb::SaveAcpConfigurationRequest {
                    profile: baseline.profile.clone(),
                    revision: baseline.revision.clone(),
                    policy_json: serde_json::to_vec(&policy).expect("serializable policy"),
                }),
                window,
                cx,
            ),
            Err(error) => {
                self.message = error;
                cx.notify();
            }
        }
    }
}
