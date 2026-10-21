use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use agent_client_protocol::LineDirection;
use serde_json::Value;

#[derive(Default)]
pub(super) struct Activity {
    state: Mutex<State>,
}

struct State {
    last: Instant,
    sealed: bool,
    active: usize,
    queued: usize,
    outgoing: HashSet<String>,
    incoming: HashSet<String>,
    urls: HashSet<String>,
    url_requests: HashMap<String, String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last: Instant::now(),
            sealed: false,
            active: 0,
            queued: 0,
            outgoing: HashSet::new(),
            incoming: HashSet::new(),
            urls: HashSet::new(),
            url_requests: HashMap::new(),
        }
    }
}

impl Activity {
    pub(super) fn observe(&self, line: &str, direction: LineDirection) {
        if line.trim().is_empty() {
            return;
        }
        // Refresh and classify under the same lock as claim, including unknown
        // responses and malformed lines. Parsing outside it opens an idle gap.
        let mut state = self.state.lock().expect("activity lock poisoned");
        state.last = Instant::now();
        if state.sealed || matches!(direction, LineDirection::Stderr) {
            return;
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let id = message
            .get("id")
            .filter(|id| !id.is_null())
            .map(Value::to_string);
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            match direction {
                LineDirection::Stdin => {
                    if let Some(id) = id {
                        state.outgoing.insert(id);
                    }
                }
                LineDirection::Stdout => {
                    // The untyped callback also admits controls and unknown
                    // notifications. Protocol-invalid messages rejected before
                    // dispatch retain their slots conservatively.
                    state.queued += 1;
                    if let Some(id) = &id {
                        state.incoming.insert(id.clone());
                    }
                    let params = &message["params"];
                    if method == "elicitation/create" && params["mode"] == "url" {
                        if let Some(url) = params["elicitationId"].as_str() {
                            state.urls.insert(url.to_owned());
                            if let Some(id) = id {
                                state.url_requests.insert(id, url.to_owned());
                            }
                        }
                    } else if method == "elicitation/complete"
                        && let Some(url) = params["elicitationId"].as_str()
                    {
                        state.urls.remove(url);
                    }
                }
                LineDirection::Stderr => {}
            }
        } else if (message.get("result").is_some() || message.get("error").is_some())
            && let Some(id) = id
        {
            match direction {
                LineDirection::Stdout => {
                    state.outgoing.remove(&id);
                }
                LineDirection::Stdin => {
                    state.incoming.remove(&id);
                    if let Some(url) = state.url_requests.remove(&id) {
                        // Missing/malformed actions retain the URL conservatively.
                        if message.get("error").is_some()
                            || matches!(
                                message["result"]["action"].as_str(),
                                Some("decline" | "cancel")
                            )
                        {
                            state.urls.remove(&url);
                        }
                    }
                }
                LineDirection::Stderr => {}
            }
        }
    }

    pub(super) fn enter(self: &Arc<Self>) -> Option<Guard> {
        self.begin(true)
    }

    pub(super) fn outbound(self: &Arc<Self>) -> Option<Guard> {
        self.begin(false)
    }

    /// Publish checkout authority only after every pre-checkout callback drained.
    /// The wire observer uses this same lock, so queued warm traffic cannot
    /// acquire the newly published Host authority.
    pub(super) fn try_bind(&self, bind: impl FnOnce()) -> bool {
        let state = self.state.lock().expect("activity lock poisoned");
        if state.sealed
            || state.active != 0
            || state.queued != 0
            || !state.incoming.is_empty()
            || !state.outgoing.is_empty()
        {
            return false;
        }
        bind();
        true
    }

    fn begin(self: &Arc<Self>, inbound: bool) -> Option<Guard> {
        let mut state = self.state.lock().expect("activity lock poisoned");
        if state.sealed {
            return None;
        }
        state.last = Instant::now();
        if inbound {
            state.queued = state.queued.saturating_sub(1);
        }
        state.active += 1;
        Some(Guard {
            activity: Arc::clone(self),
        })
    }

    pub(super) fn try_claim(&self, timeout: Duration) -> bool {
        let mut state = self.state.lock().expect("activity lock poisoned");
        if state.sealed
            || state.last.elapsed() < timeout
            || state.active != 0
            || state.queued != 0
            || !state.outgoing.is_empty()
            || !state.incoming.is_empty()
            || !state.urls.is_empty()
        {
            return false;
        }
        state.sealed = true;
        true
    }
}

pub(super) struct Guard {
    activity: Arc<Activity>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut state = self.activity.state.lock().expect("activity lock poisoned");
        state.active -= 1;
        state.last = Instant::now();
    }
}

#[cfg(test)]
mod tests;
