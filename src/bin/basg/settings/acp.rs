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

mod persistence;
use persistence::{Feedback, PendingWrite};
mod pools;
use pools::PoolRow;
#[cfg(all(test, feature = "gui-tests"))]
mod tests;
mod view;
use super::search::{Match, Query, Section};

const RETIREMENT: Section = Section::heading("session-management-retirement");
const PREWARMING: Section = Section::heading("session-management-prewarming");
const IDLE: Section = Section::new("acp-policy-idle", "acp-policy-idle-help");
const DEFAULT_AGENT: Section = Section::new("acp-policy-default", "acp-policy-default-help");
const SIZE: Section = Section::new("acp-policy-size", "acp-policy-size-help");
const POOLS: Section = Section::new("acp-policy-pools", "acp-policy-pools-help");

pub(super) fn search(query: &Query) -> Option<Match> {
    [RETIREMENT, PREWARMING, IDLE, DEFAULT_AGENT, SIZE, POOLS]
        .into_iter()
        .filter_map(|section| {
            section.score_under(query, &[super::Category::SessionManagement.section()])
        })
        .max()
}

#[derive(Default)]
struct Pages(std::collections::HashMap<Option<bas::service::tls::ConnectionOptions>, Entity<Page>>);
impl gpui_kit::Global for Pages {}

pub(super) fn shared(window: &mut Window, cx: &mut App) -> Entity<Page> {
    use gpui_kit::AppContext;
    let target = cx.try_global::<Service>().map(Service::address);
    if let Some(page) = cx
        .try_global::<Pages>()
        .and_then(|pages| pages.0.get(&target))
    {
        return page.clone();
    }
    let page = cx.new(|cx| Page::new(window, cx));
    if !cx.has_global::<Pages>() {
        cx.set_global(Pages::default());
    }
    cx.update_global::<Pages, _>(|pages, _| {
        pages.0.insert(target, page.clone());
    });
    page
}

pub(super) struct Page {
    service: Option<Service>,
    baseline: Option<pb::AcpConfiguration>,
    policy: Option<AcpLifecycle>,
    agents: Vec<AgentRegistration>,
    idle: Entity<InputState>,
    size: Entity<InputState>,
    default_agent: Option<AgentId>,
    pools: Vec<PoolRow>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    load_task: Option<Task<()>>,
    pending: Option<Task<()>>,
    write: Option<PendingWrite>,
    flush_after_write: bool,
    feedback: Feedback,
    subscriptions: Vec<Subscription>,
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
            load_task: None,
            pending: None,
            write: None,
            flush_after_write: false,
            feedback: Feedback::Idle,
            subscriptions: Vec::new(),
        };
        for input in [&page.idle, &page.size] {
            page.subscriptions
                .push(cx.subscribe(input, |this, _, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.changed(cx);
                    } else if matches!(event, InputEvent::PressEnter { .. }) {
                        this.feedback = Feedback::Idle;
                        this.flush_pending(cx);
                    }
                }));
        }
        page.subscriptions.push(cx.on_app_quit(|this, cx| {
            this.prepare_to_leave(cx);
            let write = this.write.clone();
            async move {
                if let Some(write) = write {
                    write.await;
                }
            }
        }));
        page
    }

    pub(super) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        if !self.dirty(cx) && self.write.is_none() {
            self.load(window, cx);
        }
    }

    pub(super) fn ensure_loaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.baseline.is_none() {
            self.load(window, cx);
        }
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
        self.pools = policy
            .pools
            .iter()
            .map(|(id, size)| PoolRow::new(id.clone(), *size, window, cx))
            .collect();
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
        for pool in &self.pools {
            pools.insert(pool.agent.clone(), number(&pool.size.read(cx).value())?);
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
        self.baseline.is_none() || self.load_task.is_some()
    }
}
