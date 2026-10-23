//! Agent-owned authentication UI, shared by the workspace and Agent settings.
use gpui_kit::component::*;
use std::collections::BTreeMap;

use bas::{
    authentication::{Execution, Method},
    config::AgentSpec,
};
use futures::StreamExt;
use gds_uim::el;
use gpui_kit::{
    AnyWindowHandle, Context, Entity, EventEmitter, FontWeight, Render, Subscription, Task,
    TestSupportExt, Window, div, prelude::*,
};
use tokio::sync::mpsc;

use crate::agents::{Status, check};
use crate::ui::{self, Button, feedback, tooltip::wrap_tokens};
use crate::{elicitation, i18n::tr};

mod service;
pub(crate) mod terminal;

pub(crate) struct Completed(pub Status);

pub(crate) struct Authentication {
    service: Option<check::Service>,
    spec: AgentSpec,
    window: AnyWindowHandle,
    methods: Vec<Method>,
    status: Status,
    message: String,
    forms: BTreeMap<String, Entity<elicitation::Panel>>,
    input: Option<mpsc::UnboundedSender<service::Control>>,
    generation: u64,
    _task: Option<Task<()>>,
    _terminal: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Completed> for Authentication {}

impl Authentication {
    #[cfg(all(test, feature = "gui-tests"))]
    pub(crate) fn has_pending_forms(&self) -> bool {
        !self.forms.is_empty()
    }
    pub(crate) fn new(
        spec: AgentSpec,
        status: Status,
        window: AnyWindowHandle,
        service: Option<check::Service>,
    ) -> Self {
        let methods = match &status {
            Status::AuthenticationRequired { methods, .. } => methods.clone(),
            _ => Vec::new(),
        };
        let message = match &status {
            Status::Failed(message) | Status::Missing(message) => message.clone(),
            _ => String::new(),
        };
        Self {
            service,
            spec,
            window,
            methods,
            status,
            message,
            forms: BTreeMap::new(),
            input: None,
            generation: 0,
            _task: None,
            _terminal: None,
            _subscriptions: Vec::new(),
        }
    }

    fn begin(&mut self, method: Method, cx: &mut Context<Self>) {
        if self.input.is_some()
            || !self.methods.contains(&method)
            || matches!(method.execution, Execution::Unsupported { .. })
        {
            return;
        }
        let Some(service) = self.service.clone() else {
            self.message = tr!("auth-unavailable").into();
            self.status = Status::Failed(self.message.clone());
            cx.emit(Completed(self.status.clone()));
            cx.notify();
            return;
        };
        let service::Stream {
            input,
            mut events,
            task,
        } = service::start(service, self.spec.clone(), method);
        self.generation += 1;
        let generation = self.generation;
        self._terminal.take();
        self.input = Some(input);
        self.status = Status::Authenticating;
        cx.emit(Completed(self.status.clone()));
        self.message = tr!("auth-waiting").into();
        self.forms.clear();
        self._subscriptions.clear();
        let target = self.window;
        self._task = Some(cx.spawn(async move |view, cx| {
            let _task = task;
            while let Some(event) = events.next().await {
                let ended = match &event {
                    Err(_) => true,
                    Ok(event) => matches!(event, service::Event::Check(_)),
                };
                let _ = target.update(cx, |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        if this.generation == generation {
                            this.event(event, window, cx);
                        }
                    });
                });
                if ended {
                    return;
                }
            }
        }));
        cx.notify();
    }

    fn event(
        &mut self,
        event: Result<service::Event, service::Failure>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use service::Event;
        match event {
            Err(error) => {
                self.clear_interaction();
                self.message = error.presentation();
                self.status = Status::Failed(self.message.clone());
                cx.emit(Completed(self.status.clone()));
            }
            Ok(event) => match event {
                Event::Progress(message) => self.message = message,
                Event::Check(status) => {
                    self.clear_interaction();
                    self.status = status;
                    if let Status::AuthenticationRequired { methods, .. } = &self.status {
                        self.methods = methods.clone();
                    }
                    if self.status.connected() {
                        self.message = tr!("auth-complete").into();
                    } else {
                        self.message = match &self.status {
                            Status::Failed(message) | Status::Missing(message) => message.clone(),
                            _ => tr!("auth-unconfirmed").into(),
                        };
                    }
                    cx.emit(Completed(self.status.clone()));
                }
                Event::Elicitation {
                    request_id,
                    request,
                } => match request {
                    Ok(request) => {
                        let generation = self.generation;
                        let panel = cx.new(|cx| {
                            elicitation::Panel::new(request_id.clone(), request, window, cx)
                        });
                        self._subscriptions.push(cx.subscribe(
                            &panel,
                            move |this, _, answer: &elicitation::Answered, cx| {
                                if this.generation != generation
                                    || !this.forms.contains_key(&answer.request_id)
                                {
                                    return;
                                }
                                let input = service::Control::ElicitationResponse {
                                    request_id: answer.request_id.clone(),
                                    response: answer.response.clone(),
                                };
                                if this
                                    .input
                                    .as_ref()
                                    .is_none_or(|send| send.send(input).is_err())
                                {
                                    this.message = tr!("auth-request-ended").into();
                                }
                                cx.notify();
                            },
                        ));
                        self.forms.insert(request_id, panel);
                    }
                    Err(error) => {
                        self.message =
                            tr!("elicitation-display-error", error = error.presentation());
                    }
                },
                Event::ElicitationComplete(value) => {
                    if let Some(value) = value {
                        self.forms
                            .retain(|_, panel| !panel.read(cx).completed_by(&value));
                        self.message = tr!("auth-verifying").into();
                    }
                }
                Event::ElicitationCancelled(request_id) => {
                    self.forms.remove(&request_id);
                }
                Event::Terminal(result) => {
                    let Some(service) = self.service.clone() else {
                        return;
                    };
                    match result {
                        Ok(plan) if service.address().is_local() => {
                            self.message = tr!("auth-terminal-open").into();
                            let request = service::terminal(&service.runtime(), plan);
                            let generation = self.generation;
                            self._terminal = Some(cx.spawn(async move |view, cx| {
                                let result = request.await;
                                let _ = view.update(cx, |this, cx| {
                                    this.terminal_completed(generation, result, cx);
                                    cx.notify();
                                });
                            }));
                        }
                        _ => {
                            self.message = tr!("auth-remote-terminal").into();
                            self.cancel(cx);
                        }
                    }
                }
                Event::Unknown => self.message = tr!("auth-invalid-event").into(),
            },
        }
        cx.notify();
    }

    fn terminal_completed(
        &mut self,
        generation: u64,
        result: Result<i32, String>,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation || self.input.is_none() {
            return;
        }
        let exit_code = match result {
            Ok(code) => {
                if code != 0 {
                    self.message = tr!("auth-exit-failed", code = (code).to_string());
                }
                code
            }
            Err(error) => {
                self.message = error;
                1
            }
        };
        if let Some(input) = &self.input {
            let _ = input.send(service::Control::TerminalResult { exit_code });
        }
        cx.notify();
    }

    /// Release interaction state without dropping the task delivering the final status.
    fn clear_interaction(&mut self) {
        self.input = None;
        self.forms.clear();
        self._subscriptions.clear();
        self._terminal.take();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self._task.take();
        self.clear_interaction();
        self.status = Status::AuthenticationRequired {
            program: String::new(),
            methods: self.methods.clone(),
        };
        self.message = tr!("auth-cancelled").into();
        cx.emit(Completed(self.status.clone()));
        cx.notify();
    }
}

impl Authentication {
    fn on_cancel(&mut self, _: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel(cx);
    }

    fn render_method(
        &self,
        index: usize,
        method: &Method,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let selected = method.clone();
        let unsupported = match &method.execution {
            Execution::Unsupported { reason } => Some(reason.clone()),
            _ => None,
        };
        let description = method.description.as_ref().map(|description| {
            el! {
                .div {
                    # format!("authentication-description-{index}");
                    test_support; text_size: ui::SMALL;
                    text_color: $cx.theme().muted_foreground;
                    pl_2; min_w_0; whitespace_normal;
                } [description.wrap_tokens]
            }
        });
        let reason = unsupported.as_ref().map(|reason| {
            el! {
                .div {
                    text_size: ui::SMALL;
                    text_color: $cx.theme().muted_foreground; pl_2;
                } [reason.wrap_tokens]
            }
        });
        el! {
            .div {
                # format!("authentication-choice-{index}");
                test_support; flex; flex_col; gap_2; min_w_0; max_w_full;
                @if (method.description.is_some() || unsupported.is_some()) w_full; @end;
            } [
                .div { flex; items_start; } [
                    format!("authentication-method-{index}").Button::new {
                        small; label: ~$(method.name); disabled: $unsupported.is_some();
                        on_click: $cx.listener(move |this, _, _, cx| this.begin(selected.clone(), cx));
                    }
                ],
                ..description,
                ..reason
            ]
        }.into_any_element()
    }

    fn status_label(&self) -> &'static str {
        match &self.status {
            Status::Authenticating => tr!("auth-active"),
            Status::Connected(_) => tr!("auth-done"),
            Status::Failed(_) | Status::Missing(_) => tr!("auth-incomplete"),
            Status::AuthenticationRequired { .. } => tr!("auth-required"),
            Status::Checking | Status::Connecting => tr!("auth-loading-methods"),
            Status::Installed(_) => tr!("auth-choose-method"),
        }
    }
}

impl Render for Authentication {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.input.is_some();
        let failed = matches!(self.status, Status::Failed(_) | Status::Missing(_));
        let status_color = if failed {
            cx.theme().danger
        } else if busy || self.status.connected() {
            cx.theme().primary
        } else {
            cx.theme().foreground
        };
        let message = (!self.message.is_empty()).then(|| {
            feedback(
                if failed {
                    "authentication-error"
                } else {
                    "authentication-progress"
                },
                self.message.clone(),
                if failed {
                    cx.theme().danger
                } else {
                    cx.theme().foreground
                },
            )
        });
        let methods = (!busy && !self.status.connected()).then(|| {
            let empty = self.methods.is_empty().then(|| {
                feedback("authentication-no-methods", tr!("auth-no-methods"), cx.theme().muted_foreground)
            });
            let choices = self
                .methods
                .iter()
                .enumerate()
                .map(|(index, method)| self.render_method(index, method, cx))
                .collect::<Vec<_>>();
            el! {
                .div {
                    # "authentication-methods"; test_support; flex; flex_wrap; items_start; gap_3; pt_1;
                } [ ..empty, ..choices ]
            }
        });
        el! {
            .div {
                # "authentication-panel"; test_support; flex; flex_col; gap_3;
                min_w_0; text_size: ui::SMALL;
            } [
                .div { flex; items_center; gap_2; } [
                    .div {
                        text_size: ui::SMALL; font_weight: FontWeight::SEMIBOLD;
                    } [tr!("auth-agent-title", agent = (self.spec.id).to_string())],
                    .div {
                        # "authentication-state"; test_support; flex_shrink_0;
                        text_size: ui::SMALL; text_color: status_color;
                        bg: $cx.theme().muted; rounded_sm; px_2; py_1;
                    } [ $self.status_label() ]
                ],
                ..message,
                ..methods,
                ..$self.forms.values().cloned(),
                @if (busy)
                    .div { flex; } [
                        "authentication-cancel".Button::new {
                            small; label: tr!("auth-cancel");
                            on_click: (cx, Self::on_cancel).Context::listener;
                        },
                    ],
                @end,
            ]
        }
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
