//! Only explicitly configured reserves become editable rows.
use super::*;

pub(super) struct PoolRow {
    pub agent: AgentId,
    pub size: Entity<InputState>,
    _subscription: Subscription,
}

impl PoolRow {
    pub(super) fn new(
        agent: AgentId,
        size: usize,
        window: &mut Window,
        cx: &mut Context<Page>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(size.to_string()));
        let subscription = cx.subscribe(&input, |page, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                page.changed(cx);
            } else if matches!(event, InputEvent::PressEnter { .. }) {
                page.feedback = Feedback::Idle;
                page.flush_pending(cx);
            }
        });
        Self {
            agent,
            size: input,
            _subscription: subscription,
        }
    }
}

impl Page {
    pub(super) fn acp_agents(&self) -> impl Iterator<Item = &AgentId> {
        self.agents
            .iter()
            .filter(|agent| agent.as_custom().is_none_or(|spec| spec.kind == Kind::Acp))
            .map(AgentRegistration::id)
    }

    pub(super) fn pool_candidates(&self) -> Vec<AgentId> {
        self.acp_agents()
            .filter(|id| !self.pools.iter().any(|pool| &pool.agent == *id))
            .cloned()
            .collect()
    }

    pub(super) fn add_pool(&mut self, id: AgentId, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled() || !self.pool_candidates().contains(&id) {
            return;
        }
        let row = PoolRow::new(id, AcpLifecycle::default().pool_size, window, cx);
        row.size.update(cx, |input, cx| input.focus(window, cx));
        self.pools.push(row);
        self.changed(cx);
    }

    pub(super) fn remove_pool(&mut self, id: &AgentId, cx: &mut Context<Self>) {
        if self.disabled() {
            return;
        }
        if let Some(index) = self.pools.iter().position(|pool| &pool.agent == id) {
            self.pools.remove(index);
            self.changed(cx);
        }
    }
}
