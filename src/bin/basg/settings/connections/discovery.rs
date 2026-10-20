use super::*;
use bas::service::{Client, Target, pb};
use std::time::Duration;
use tokio_util::task::AbortOnDropHandle;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const LOCAL_NAME: &str = "Local";

#[cfg(test)]
mod tests;

pub(super) enum State {
    Loading,
    Found(Target),
    Failed,
}

/// # Errors
/// Fails if authenticated IPC or the read-only Bas RPC cannot complete in time.
async fn probe(candidate: Target) -> Result<Target, ()> {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let endpoint = candidate.resolved().map_err(|_| ())?;
        let mut client = Client::connect_target(&candidate).await.map_err(|_| ())?;
        client
            .list_agents(pb::ListAgentsRequest {})
            .await
            .map_err(|_| ())?;
        Ok(endpoint)
    })
    .await
    .map_err(|_| ())?
}

impl Page {
    pub(super) fn dismiss_discovery(&mut self) {
        self.scan = None;
        self.scan_generation += 1;
        self.discovery = None;
    }

    pub(super) fn discover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        let Some(service) = cx.try_global::<crate::agents::catalog::Service>() else {
            self.begin_scan(std::future::ready(Err(())), window, cx);
            return;
        };
        // Only borrow the application's runtime, never its target or credentials.
        let task = AbortOnDropHandle::new(service.runtime().spawn(probe(Target::Local)));
        self.begin_scan(async move { task.await.unwrap_or(Err(())) }, window, cx);
    }

    pub(super) fn begin_scan(
        &mut self,
        request: impl Future<Output = Result<Target, ()>> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_discovery();
        self.discovery = Some(State::Loading);
        self.feedback = Feedback::Idle;
        self.scroll_to = Some(0);
        self.focus.focus(window, cx);
        let generation = self.scan_generation;
        self.scan = Some(cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                page.finish_scan(generation, result, cx);
            });
        }));
        cx.notify();
    }

    pub(super) fn finish_scan(
        &mut self,
        generation: u64,
        result: Result<Target, ()>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.scan_generation || self.discovery.is_none() || self.editor.is_some() {
            return;
        }
        self.discovery = Some(match result {
            Ok(endpoint) => State::Found(endpoint),
            Err(()) => State::Failed,
        });
        cx.notify();
    }

    pub(super) fn local_profile<'a>(
        &self,
        endpoint: &Target,
        cx: &'a App,
    ) -> Option<&'a ConnectionProfile> {
        self.profiles.read(cx).entries().iter().find(|profile| {
            profile.target == Target::Local
                || profile
                    .target
                    .resolved()
                    .is_ok_and(|target| target == *endpoint)
        })
    }

    pub(super) fn add_discovered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(State::Found(endpoint)) = &self.discovery else {
            return;
        };
        if self.local_profile(endpoint, cx).is_some() {
            return;
        }
        let profile = ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: LOCAL_NAME.into(),
            note: None,
            target: Target::Local,
            tls: None,
        };
        match self
            .profiles
            .update(cx, |profiles, cx| profiles.put(profile, cx))
        {
            Ok(_) => {
                self.dismiss_discovery();
                self.feedback = Feedback::Saved;
                self.focus.focus(window, cx);
            }
            Err(error) => self.feedback = Feedback::Error(error),
        }
        cx.notify();
    }
}
