//! Agent-owned authentication UI, shared by the workspace and Agent settings.
use gpui_kit::component::*;
use std::{collections::BTreeMap, path::PathBuf};

use basr::{
    authentication::{Execution, Method},
    config::AgentSpec,
    service::pb,
};
use futures::StreamExt;
use gpui_kit::{
    AnyWindowHandle, Context, Entity, EventEmitter, FontWeight, Render, Subscription, Task,
    TestSupportExt, Window, component::button::Button, div, prelude::*,
};
use tokio::sync::mpsc;

use crate::settings::agents::check::{self, Status};

pub(crate) mod elicitation;
mod service;
pub(crate) mod terminal;

pub(crate) struct Completed(pub Status);

pub(crate) struct Authentication {
    spec: AgentSpec,
    settings_path: PathBuf,
    window: AnyWindowHandle,
    methods: Vec<Method>,
    status: Status,
    message: String,
    forms: BTreeMap<String, Entity<elicitation::Panel>>,
    input: Option<mpsc::UnboundedSender<pb::AuthenticationInput>>,
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
        settings_path: PathBuf,
        window: AnyWindowHandle,
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
            spec,
            settings_path,
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
        let Some(service) = cx.try_global::<check::Service>().cloned() else {
            self.message = crate::i18n::tr!("auth-unavailable").into();
            self.status = Status::Failed(self.message.clone());
            cx.emit(Completed(self.status.clone()));
            cx.notify();
            return;
        };
        let service::Stream {
            input,
            mut events,
            task,
        } = service::start(
            service,
            self.spec.clone(),
            method,
            self.settings_path.clone(),
        );
        self.generation += 1;
        let generation = self.generation;
        self._terminal.take();
        self.input = Some(input);
        self.status = Status::Authenticating;
        cx.emit(Completed(self.status.clone()));
        self.message = crate::i18n::tr!("auth-waiting").into();
        self.forms.clear();
        self._subscriptions.clear();
        let target = self.window;
        self._task = Some(cx.spawn(async move |view, cx| {
            let _task = task;
            while let Some(event) = events.next().await {
                let ended = match &event {
                    Err(_) => true,
                    Ok(event) => {
                        matches!(event.kind, Some(pb::authentication_event::Kind::Check(_)))
                    }
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
        event: Result<pb::AuthenticationEvent, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use pb::authentication_event::Kind;
        match event {
            Err(error) => {
                self.input = None;
                self.forms.clear();
                self._subscriptions.clear();
                self._terminal.take();
                self.message = error.clone();
                self.status = Status::Failed(error);
                cx.emit(Completed(self.status.clone()));
            }
            Ok(event) => match event.kind {
                Some(Kind::Progress(message)) => self.message = message,
                Some(Kind::Check(check)) => {
                    self.input = None;
                    self.forms.clear();
                    self._subscriptions.clear();
                    self._terminal.take();
                    self.status = check::decode_response(check).unwrap_or_else(Status::Failed);
                    if let Status::AuthenticationRequired { methods, .. } = &self.status {
                        self.methods = methods.clone();
                    }
                    if self.status.connected() {
                        self.message = crate::i18n::tr!("auth-complete").into();
                        self.forms.clear();
                    } else {
                        self.message = match &self.status {
                            Status::Failed(message) | Status::Missing(message) => message.clone(),
                            _ => crate::i18n::tr!("auth-unconfirmed").into(),
                        };
                    }
                    cx.emit(Completed(self.status.clone()));
                }
                Some(Kind::Elicitation(pending)) => {
                    let request = pending
                        .request
                        .as_ref()
                        .ok_or_else(|| crate::i18n::tr!("auth-empty-request").to_owned())
                        .and_then(|payload| {
                            payload
                                .decode::<serde_json::Value>()
                                .map_err(|error| error.to_string())
                        });
                    match request.and_then(|raw| {
                        basr::authentication::elicitation::parse_request(&raw)
                            .map_err(|error| error.to_string())
                    }) {
                        Ok(request) => {
                            let request_id = pending.request_id;
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
                                    let input = pb::AuthenticationInput {
                                        kind: Some(
                                            pb::authentication_input::Kind::ElicitationResponse(
                                                pb::ElicitationAnswer {
                                                    request_id: answer.request_id.clone(),
                                                    response: Some(pb::AcpPayload {
                                                        json: serde_json::to_vec(&answer.response)
                                                            .expect("JSON value serializes"),
                                                    }),
                                                },
                                            ),
                                        ),
                                    };
                                    if this
                                        .input
                                        .as_ref()
                                        .is_none_or(|send| send.send(input).is_err())
                                    {
                                        this.message =
                                            crate::i18n::tr!("auth-request-ended").into();
                                    }
                                    cx.notify();
                                },
                            ));
                            self.forms.insert(request_id, panel);
                        }
                        Err(error) => {
                            self.message = crate::i18n::tr!(
                                "elicitation-display-error",
                                error = (error).to_string()
                            );
                        }
                    }
                }
                Some(Kind::ElicitationComplete(payload)) => {
                    if let Ok(value) = payload.decode::<serde_json::Value>() {
                        self.forms
                            .retain(|_, panel| !panel.read(cx).completed_by(&value));
                        self.message = crate::i18n::tr!("auth-verifying").into();
                    }
                }
                Some(Kind::ElicitationCancelled(request_id)) => {
                    self.forms.remove(&request_id);
                }
                Some(Kind::Terminal(payload)) => {
                    let result = payload.decode::<basr::authentication::Invocation>();
                    let Some(service) = cx.try_global::<check::Service>().cloned() else {
                        return;
                    };
                    match result {
                        Ok(plan) if service.address().ip().is_loopback() => {
                            self.message = crate::i18n::tr!("auth-terminal-open").into();
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
                            self.message = crate::i18n::tr!("auth-remote-terminal").into();
                            self.cancel(cx);
                        }
                    }
                }
                None => self.message = crate::i18n::tr!("auth-invalid-event").into(),
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
                    self.message = crate::i18n::tr!("auth-exit-failed", code = (code).to_string());
                }
                code
            }
            Err(error) => {
                self.message = error;
                1
            }
        };
        if let Some(input) = &self.input {
            let _ = input.send(pb::AuthenticationInput {
                kind: Some(pb::authentication_input::Kind::TerminalResult(
                    pb::TerminalAuthenticationResult { exit_code },
                )),
            });
        }
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self._task.take();
        self._terminal.take();
        self.input = None;
        self.forms.clear();
        self._subscriptions.clear();
        self.status = Status::AuthenticationRequired {
            program: String::new(),
            methods: self.methods.clone(),
        };
        self.message = crate::i18n::tr!("auth-cancelled").into();
        cx.emit(Completed(self.status.clone()));
        cx.notify();
    }
}

impl Authentication {
    fn status_label(&self) -> &'static str {
        match &self.status {
            Status::Authenticating => crate::i18n::tr!("auth-active"),
            Status::Connected(_) => crate::i18n::tr!("auth-done"),
            Status::Failed(_) | Status::Missing(_) => crate::i18n::tr!("auth-incomplete"),
            Status::AuthenticationRequired { .. } => crate::i18n::tr!("auth-required"),
            Status::Checking | Status::Connecting => crate::i18n::tr!("auth-loading-methods"),
            Status::Installed(_) => crate::i18n::tr!("auth-choose-method"),
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
        div()
            .id("authentication-panel")
            .test_support()
            .flex()
            .flex_col()
            .gap_3()
            .min_w_0()
            .text_size(crate::ui::SMALL)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(crate::ui::SMALL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::i18n::tr!(
                                "auth-agent-title",
                                agent = (self.spec.id).to_string()
                            )),
                    )
                    .child(
                        div()
                            .id("authentication-state")
                            .test_support()
                            .flex_shrink_0()
                            .text_size(crate::ui::SMALL)
                            .text_color(status_color)
                            .bg(cx.theme().muted)
                            .rounded_sm()
                            .px_2()
                            .py_1()
                            .child(self.status_label()),
                    ),
            )
            .when(!self.message.is_empty(), |panel| {
                panel.child(
                    div()
                        .id(if failed {
                            "authentication-error"
                        } else {
                            "authentication-progress"
                        })
                        .test_support()
                        .min_w_0()
                        .whitespace_normal()
                        .when(failed, |message| {
                            message
                                .text_size(crate::ui::SMALL)
                                .text_color(cx.theme().danger)
                        })
                        .when(!failed, |message| {
                            message
                                .text_size(crate::ui::SMALL)
                                .text_color(cx.theme().foreground)
                        })
                        .child(crate::tooltip::wrap_tokens(&self.message)),
                )
            })
            .when(!busy && !self.status.connected(), |panel| {
                panel.child(
                    div()
                        .id("authentication-methods")
                        .test_support()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .pt_1()
                        .when(self.methods.is_empty(), |panel| {
                            panel.child(
                                div()
                                    .text_size(crate::ui::SMALL)
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::i18n::tr!("auth-no-methods")),
                            )
                        })
                        .children(self.methods.iter().enumerate().map(|(index, method)| {
                            let selected = method.clone();
                            let unsupported = match &method.execution {
                                Execution::Unsupported { reason } => Some(reason.clone()),
                                _ => None,
                            };
                            div()
                                .id(format!("authentication-choice-{index}"))
                                .test_support()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div().flex().items_start().child(
                                        Button::new(format!("authentication-method-{index}"))
                                            .small()
                                            .label(method.name.clone())
                                            .disabled(unsupported.is_some())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.begin(selected.clone(), cx)
                                            })),
                                    ),
                                )
                                .when_some(method.description.clone(), |row, description| {
                                    row.child(
                                        div()
                                            .id(format!("authentication-description-{index}"))
                                            .test_support()
                                            .text_size(crate::ui::SMALL)
                                            .text_color(cx.theme().muted_foreground)
                                            .pl_2()
                                            .min_w_0()
                                            .whitespace_normal()
                                            .child(crate::tooltip::wrap_tokens(&description)),
                                    )
                                })
                                .when_some(unsupported, |row, reason| {
                                    row.child(
                                        div()
                                            .text_size(crate::ui::SMALL)
                                            .text_color(cx.theme().muted_foreground)
                                            .pl_2()
                                            .child(crate::tooltip::wrap_tokens(&reason)),
                                    )
                                })
                        })),
                )
            })
            .children(self.forms.values().cloned())
            .when(busy, |panel| {
                panel.child(
                    div().flex().child(
                        Button::new("authentication-cancel")
                            .small()
                            .label(crate::i18n::tr!("auth-cancel"))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
                )
            })
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
