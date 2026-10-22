//! Debounced preferences, with one revision-checked write at a time.
use super::*;
use futures::FutureExt;

pub(super) type PendingWrite = futures::future::Shared<Task<()>>;

pub(super) enum Feedback {
    Idle,
    Saved,
    Error(String),
}

impl Page {
    pub(in crate::settings) fn status(&self, cx: &App) -> String {
        if let Some(error) = self.error(cx) {
            return error;
        }
        if self.load_task.is_some() || self.write.is_some() || self.pending.is_some() {
            return crate::i18n::tr!("acp-policy-loading").into();
        }
        if self.dirty(cx) {
            return crate::i18n::tr!("settings-unsaved").into();
        }
        match self.feedback {
            Feedback::Saved => crate::i18n::tr!("settings-applied").into(),
            _ => String::new(),
        }
    }

    pub(super) fn error(&self, cx: &App) -> Option<String> {
        self.baseline
            .as_ref()
            .and_then(|_| self.candidate(cx).err())
            .or_else(|| {
                if let Feedback::Error(error) = &self.feedback {
                    Some(error.clone())
                } else {
                    None
                }
            })
    }

    pub(super) fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.load_task.is_some() || self.write.is_some() {
            return;
        }
        self.pending = None;
        self.flush_after_write = false;
        let Some(service) = &self.service else {
            self.feedback = Feedback::Error(crate::i18n::tr!("acp-policy-no-service").into());
            cx.notify();
            return;
        };
        let request = service.acp_configuration(None);
        self.feedback = Feedback::Idle;
        self.load_task = Some(cx.spawn_in(window, async move |page, cx| {
            let result = request.await;
            let _ = page.update_in(cx, |page, window, cx| {
                page.load_task = None;
                page.feedback = match result.and_then(|value| page.accept(value, window, cx)) {
                    Ok(()) => Feedback::Idle,
                    Err(error) => Feedback::Error(error),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn changed(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        if self.write.is_none() {
            self.flush_after_write = false;
        }
        if !self.dirty(cx) && self.write.is_none() {
            cx.notify();
            return;
        }
        self.feedback = Feedback::Idle;
        if !self.disabled() && self.dirty(cx) && self.candidate(cx).is_ok() {
            self.pending = Some(cx.spawn(async move |page, cx| {
                cx.background_executor()
                    .timer(super::super::SAVE_DELAY)
                    .await;
                let _ = page.update(cx, |page, cx| page.save(cx));
            }));
        }
        cx.notify();
    }

    pub(in crate::settings) fn flush_pending(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        self.flush_after_write = true;
        self.save(cx);
    }

    pub(in crate::settings) fn prepare_to_leave(&mut self, cx: &mut Context<Self>) {
        // Cancel window-bound reads, without saving a draft being discarded by Reload.
        if self.load_task.take().is_none() {
            self.flush_pending(cx);
        }
    }

    pub(in crate::settings) fn pending_write(&self) -> Option<PendingWrite> {
        self.write.clone()
    }

    fn save_request(&self, cx: &App) -> Result<Option<pb::SaveAcpConfigurationRequest>, String> {
        let Some(baseline) = &self.baseline else {
            return Ok(None);
        };
        let policy = self.candidate(cx)?;
        if self.policy.as_ref() == Some(&policy) {
            return Ok(None);
        }
        Ok(Some(pb::SaveAcpConfigurationRequest {
            profile: baseline.profile.clone(),
            revision: baseline.revision.clone(),
            policy_json: serde_json::to_vec(&policy).expect("serializable policy"),
        }))
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        if self.disabled() || self.write.is_some() || matches!(self.feedback, Feedback::Error(_)) {
            return;
        }
        let request = match self.save_request(cx) {
            Ok(Some(request)) => request,
            _ => {
                cx.notify();
                return;
            }
        };
        let Some(service) = self.service.clone() else {
            return;
        };
        self.feedback = Feedback::Idle;
        // Keep accepted writes alive after window close, tied to their original server.
        let page = cx.entity();
        let task = cx
            .spawn(async move |_, cx| {
                let mut request = request;
                loop {
                    let result = service.acp_configuration(Some(request)).await;
                    let next = page.update(cx, |page, cx| page.saved(result, cx));
                    match next {
                        Some(next) => request = next,
                        _ => break,
                    }
                }
            })
            .shared();
        self.write = Some(task.clone());
        cx.spawn(async move |_, _| task.await).detach();
        cx.notify();
    }

    fn saved(
        &mut self,
        result: Result<pb::AcpConfiguration, String>,
        cx: &mut Context<Self>,
    ) -> Option<pb::SaveAcpConfigurationRequest> {
        let result = result.and_then(|value| {
            let policy: AcpLifecycle =
                serde_json::from_slice(&value.policy_json).map_err(|e| e.to_string())?;
            policy.validate(&self.agents).map_err(|e| e.to_string())?;
            // Acknowledgements update the baseline, never overwrite newer form input.
            self.policy = Some(policy);
            self.baseline = Some(value);
            Ok(())
        });
        if let Err(error) = result {
            self.write = None;
            self.flush_after_write = false;
            self.pending = None;
            self.feedback = Feedback::Error(error);
            cx.notify();
            return None;
        }
        if self.flush_after_write
            && let Ok(Some(request)) = self.save_request(cx)
        {
            return Some(request);
        }
        self.write = None;
        self.flush_after_write = false;
        self.pending = None;
        self.feedback = Feedback::Saved;
        if self.dirty(cx) {
            self.changed(cx);
        }
        cx.notify();
        None
    }
}
