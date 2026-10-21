//! Idle initialized transports, keyed by the complete resolved launch spec.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{sync::Notify, task::JoinSet, time::Instant};
use tokio_util::task::AbortOnDropHandle;

use super::{super::AgentSpec, Warm};

const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct Pool {
    state: Mutex<State>,
    shutdown: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct State {
    closed: bool,
    entries: Vec<Entry>,
}

struct Entry {
    spec: Arc<AgentSpec>,
    shared: Arc<Shared>,
    // The worker owns only Shared, never Entry or Pool.
    task: AbortOnDropHandle<()>,
}

struct Shared {
    state: Mutex<Inventory>,
    changed: Notify,
}

struct Inventory {
    target: usize,
    timeout: Duration,
    closed: bool,
    idle: Vec<Warm>,
}

impl Pool {
    /// Forget initialized credential state without touching checked-out sessions.
    pub(super) fn invalidate(&self) {
        let entries = std::mem::take(&mut self.state.lock().expect("ACP pool state lock").entries);
        drop(entries);
    }

    /// Set the idle-plus-initializing capacity for this exact resolved spec.
    /// Requires a Tokio runtime; changes to the timeout affect future attempts.
    pub(super) fn maintain(&self, spec: Arc<AgentSpec>, target: usize, timeout: Duration) {
        let mut state = self.state.lock().expect("ACP pool state lock");
        if state.closed {
            return;
        }
        if let Some(entry) = state.entries.iter().find(|entry| entry.spec == spec) {
            let mut inventory = entry.shared.state.lock().expect("ACP pool inventory lock");
            inventory.target = target;
            inventory.timeout = timeout;
            inventory.idle.truncate(target);
            entry.shared.changed.notify_one();
            return;
        }
        if target == 0 {
            return;
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(Inventory {
                target,
                timeout,
                closed: false,
                idle: Vec::new(),
            }),
            changed: Notify::new(),
        });
        let task = AbortOnDropHandle::new(tokio::spawn(run(spec.clone(), shared.clone())));
        state.entries.push(Entry { spec, shared, task });
    }

    /// Checkout never waits for initialization and never returns a dead transport.
    pub(super) fn take(&self, spec: &Arc<AgentSpec>) -> Option<Warm> {
        let state = self.state.lock().expect("ACP pool state lock");
        if state.closed {
            return None;
        }
        let entry = state.entries.iter().find(|entry| entry.spec == *spec)?;
        let mut inventory = entry.shared.state.lock().expect("ACP pool inventory lock");
        let warm = std::iter::from_fn(|| inventory.idle.pop()).find(Warm::is_alive);
        entry.shared.changed.notify_one();
        warm
    }

    /// Permanently close admission, cancel and join initialization, and close idle transports.
    pub(super) async fn shutdown(&self) {
        // Cancelling shutdown aborts its current worker; remaining handles stay owned by Pool.
        let _shutdown = self.shutdown.lock().await;
        {
            let mut state = self.state.lock().expect("ACP pool state lock");
            state.closed = true;
            for entry in &state.entries {
                entry
                    .shared
                    .state
                    .lock()
                    .expect("ACP pool inventory lock")
                    .closed = true;
                entry.shared.changed.notify_one();
            }
        }
        loop {
            let entry = self
                .state
                .lock()
                .expect("ACP pool state lock")
                .entries
                .pop();
            let Some(entry) = entry else {
                break;
            };
            let _ = entry.task.await;
        }
    }
}

async fn run(spec: Arc<AgentSpec>, shared: Arc<Shared>) {
    let mut initializing = JoinSet::new();
    let mut retry_at = Instant::now();
    let mut backoff = INITIAL_BACKOFF;

    loop {
        let excess = {
            let mut inventory = shared.state.lock().expect("ACP pool inventory lock");
            if inventory.closed {
                break;
            }
            inventory.idle.retain(Warm::is_alive);
            let target = inventory.target;
            inventory.idle.truncate(target);
            let excess = inventory.idle.len() + initializing.len() > target;
            if !excess && Instant::now() >= retry_at {
                while inventory.idle.len() + initializing.len() < target {
                    let spec = spec.clone();
                    let timeout = inventory.timeout;
                    initializing.spawn(async move {
                        tokio::time::timeout(timeout, Warm::initialize(spec)).await
                    });
                }
            }
            excess
        };
        if excess {
            // Join cancelled attempts before admitting replacements at a smaller capacity.
            initializing.abort_all();
            while initializing.join_next().await.is_some() {}
            continue;
        }

        let wake_at = if retry_at > Instant::now() {
            retry_at.min(Instant::now() + HEALTH_INTERVAL)
        } else {
            Instant::now() + HEALTH_INTERVAL
        };
        tokio::select! {
            biased;
            _ = shared.changed.notified() => {}
            result = initializing.join_next(), if !initializing.is_empty() => {
                match result {
                    Some(Ok(Ok(Ok(warm)))) if warm.is_alive() => {
                        let mut inventory = shared.state.lock().expect("ACP pool inventory lock");
                        // Shutdown drains this inventory, including racing successful attempts.
                        if inventory.closed || inventory.idle.len() < inventory.target {
                            inventory.idle.push(warm);
                        }
                        backoff = INITIAL_BACKOFF;
                    }
                    _ => {
                        retry_at = Instant::now() + backoff;
                        backoff = next_backoff(backoff);
                    }
                }
            }
            _ = tokio::time::sleep_until(wake_at) => {}
        }
    }

    initializing.abort_all();
    // A completed task may already own a Warm: close it rather than just dropping it.
    while let Some(result) = initializing.join_next().await {
        if let Ok(Ok(Ok(warm))) = result {
            warm.shutdown().await;
        }
    }
    let idle = {
        let mut inventory = shared.state.lock().expect("ACP pool inventory lock");
        std::mem::take(&mut inventory.idle)
    };
    for warm in idle {
        warm.shutdown().await;
    }
}

fn next_backoff(backoff: Duration) -> Duration {
    backoff.saturating_mul(2).min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests;
