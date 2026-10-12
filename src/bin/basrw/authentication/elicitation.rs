//! The same validated form and external authorization UI is used in both flows.
use gpui_kit::component::*;
use std::collections::BTreeMap;

use basr::authentication::elicitation::{Kind, Mode, Request};
use gpui_kit::{
    App, Context, Entity, EventEmitter, Render, Subscription, TestSupportExt, Window,
    component::{
        button::*,
        checkbox::Checkbox,
        input::{Input, InputEvent, InputState},
        switch::Switch,
    },
    div,
    prelude::*,
};
use serde_json::{Map, Value, json};

pub(crate) struct Answered {
    pub request_id: String,
    pub response: Value,
}

pub(crate) struct Panel {
    pub request_id: String,
    pub request: Request,
    inputs: BTreeMap<String, Entity<InputState>>,
    values: Map<String, Value>,
    errors: BTreeMap<String, String>,
    pub submitting: bool,
    pub accepted: bool,
    message: String,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Answered> for Panel {}

impl Panel {
    pub(crate) fn new(
        request_id: String,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut inputs = BTreeMap::new();
        let mut subscriptions = Vec::new();
        let mut values = match &request.mode {
            Mode::Form(form) => form.defaults(),
            Mode::Url(_) => Map::new(),
        };
        if let Mode::Form(form) = &request.mode {
            for field in &form.fields {
                if field.required && !values.contains_key(&field.name) {
                    match field.kind {
                        Kind::Boolean => {
                            values.insert(field.name.clone(), Value::Bool(false));
                        }
                        Kind::MultiSelect { .. } => {
                            values.insert(field.name.clone(), Value::Array(Vec::new()));
                        }
                        _ => {}
                    }
                }
                if matches!(field.kind, Kind::String | Kind::Number | Kind::Integer) {
                    let text = values
                        .get(&field.name)
                        .map(|value| match value {
                            Value::String(value) => value.clone(),
                            _ => value.to_string(),
                        })
                        .unwrap_or_default();
                    let input = cx.new(|cx| {
                        let mut input = InputState::new(window, cx);
                        input.set_value(text, window, cx);
                        input
                    });
                    subscriptions
                        .push(cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify()));
                    inputs.insert(field.name.clone(), input);
                }
            }
        }
        Self {
            request_id,
            request,
            inputs,
            values,
            errors: BTreeMap::new(),
            submitting: false,
            accepted: false,
            message: String::new(),
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn completed_by(&self, value: &Value) -> bool {
        let Mode::Url(url) = &self.request.mode else {
            return false;
        };
        value.get("elicitationId").and_then(Value::as_str) == Some(url.elicitation_id.as_str())
    }

    pub(crate) fn failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.submitting = false;
        self.message = message;
        cx.notify();
    }

    fn content(&self, cx: &App) -> Result<Map<String, Value>, BTreeMap<String, String>> {
        let Mode::Form(form) = &self.request.mode else {
            return Ok(Map::new());
        };
        let mut values = self.values.clone();
        let mut errors = BTreeMap::new();
        for field in &form.fields {
            if let Some(input) = self.inputs.get(&field.name) {
                let text = input.read(cx).value().to_string();
                if text.is_empty() && !field.required {
                    values.remove(&field.name);
                    continue;
                }
                let value = match field.kind {
                    Kind::String => Ok(Value::String(text)),
                    Kind::Integer => text
                        .parse::<i64>()
                        .map(|value| json!(value))
                        .map_err(|_| crate::i18n::tr!("validation-integer").into()),
                    Kind::Number => text
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .and_then(serde_json::Number::from_f64)
                        .map(Value::Number)
                        .ok_or_else(|| crate::i18n::tr!("validation-number").into()),
                    _ => unreachable!("only textual fields create inputs"),
                };
                match value {
                    Ok(value) => {
                        values.insert(field.name.clone(), value);
                    }
                    Err(error) => {
                        errors.insert(field.name.clone(), error);
                    }
                }
            }
            match values.get(&field.name) {
                Some(value) => {
                    if let Err(error) = field.validate(value) {
                        errors.insert(field.name.clone(), error.to_string());
                    }
                }
                None if field.required => {
                    errors.insert(
                        field.name.clone(),
                        crate::i18n::tr!("validation-required", field = (field.title).to_string()),
                    );
                }
                _ => {}
            }
        }
        if errors.is_empty() {
            Ok(values)
        } else {
            Err(errors)
        }
    }

    fn answer(&mut self, action: &str, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let response = if action == "accept" && matches!(self.request.mode, Mode::Form(_)) {
            match self.content(cx) {
                Ok(content) => json!({"action":action,"content":content}),
                Err(errors) => {
                    self.errors = errors;
                    cx.notify();
                    return;
                }
            }
        } else {
            json!({"action":action})
        };
        if let Err(error) =
            basr::authentication::elicitation::validate_response(&self.request, &response)
        {
            self.message = error.to_string();
            cx.notify();
            return;
        }
        self.errors.clear();
        self.submitting = true;
        self.accepted = action == "accept";
        self.message = if matches!(self.request.mode, Mode::Url(_)) && self.accepted {
            crate::i18n::tr!("elicitation-browser").into()
        } else {
            crate::i18n::tr!("elicitation-submitting").into()
        };
        cx.emit(Answered {
            request_id: self.request_id.clone(),
            response,
        });
        cx.notify();
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div()
            .id(format!("elicitation-{}", self.request_id))
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(crate::ui::SMALL)
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(self.request.message.clone());
        match &self.request.mode {
            Mode::Form(form) => {
                for field in &form.fields {
                    let name = field.name.clone();
                    let mut row = div()
                        .id(format!("elicitation-field-{name}"))
                        .test_support()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(format!(
                            "{}{}",
                            field.title,
                            if field.required { " *" } else { "" }
                        ))
                        .when_some(field.description.clone(), |row, description| {
                            row.child(
                                div()
                                    .text_size(crate::ui::SMALL)
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description),
                            )
                        });
                    let id = format!("elicitation-input-{name}");
                    match &field.kind {
                        Kind::String | Kind::Number | Kind::Integer => {
                            row = row.child(
                                Input::new(&self.inputs[&name])
                                    .id(id)
                                    .disabled(self.submitting),
                            );
                        }
                        Kind::Boolean => {
                            let checked = self
                                .values
                                .get(&name)
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            let name = name.clone();
                            row = row.child(
                                Switch::new(id)
                                    .checked(checked)
                                    .disabled(self.submitting)
                                    .accessibility_label(field.title.clone())
                                    .on_change(cx.listener(move |this, checked, _, cx| {
                                        this.values.insert(name.clone(), Value::Bool(*checked));
                                        cx.notify();
                                    })),
                            );
                        }
                        Kind::Enum { options } => {
                            row = row.child(div().flex().flex_wrap().gap_1().children(
                                options.iter().enumerate().map(|(index, option)| {
                                    let name = name.clone();
                                    let value = option.value.clone();
                                    crate::ui::action_button(format!("{id}-{index}"))
                                        .label(option.title.clone())
                                        .disabled(self.submitting)
                                        .tooltip(crate::i18n::tr!(
                                            "option-choose",
                                            name = option.title.as_str()
                                        ))
                                        .when(
                                            self.values.get(&name).and_then(Value::as_str)
                                                == Some(value.as_str()),
                                            |button| button.primary(),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.values
                                                .insert(name.clone(), Value::String(value.clone()));
                                            cx.notify();
                                        }))
                                }),
                            ));
                            if let Some(description) = options
                                .iter()
                                .find(|option| {
                                    self.values.get(&name).and_then(Value::as_str)
                                        == Some(option.value.as_str())
                                })
                                .and_then(|option| option.description.as_ref())
                            {
                                row = row.child(
                                    div()
                                        .text_size(crate::ui::SMALL)
                                        .text_color(cx.theme().muted_foreground)
                                        .child(description.clone()),
                                );
                            }
                        }
                        Kind::MultiSelect { options } => {
                            row = row.child(div().flex().flex_col().gap_1().children(
                                options.iter().enumerate().map(|(index, option)| {
                                    let name = name.clone();
                                    let value = option.value.clone();
                                    let checked = self
                                        .values
                                        .get(&name)
                                        .and_then(Value::as_array)
                                        .is_some_and(|values| {
                                            values
                                                .iter()
                                                .any(|item| item.as_str() == Some(value.as_str()))
                                        });
                                    Checkbox::new(format!("{id}-{index}"))
                                        .label(option.title.clone())
                                        .checked(checked)
                                        .disabled(self.submitting)
                                        .on_click(cx.listener(move |this, checked, _, cx| {
                                            let entry = this
                                                .values
                                                .entry(name.clone())
                                                .or_insert_with(|| Value::Array(Vec::new()));
                                            let values = entry
                                                .as_array_mut()
                                                .expect("multi select stores an array");
                                            values.retain(|item| {
                                                item.as_str() != Some(value.as_str())
                                            });
                                            if *checked {
                                                values.push(Value::String(value.clone()));
                                            }
                                            cx.notify();
                                        }))
                                }),
                            ));
                        }
                    }
                    if let Some(error) = self.errors.get(&name) {
                        row = row.child(
                            div()
                                .text_size(crate::ui::SMALL)
                                .text_color(cx.theme().danger)
                                .child(error.clone()),
                        );
                    }
                    panel = panel.child(row);
                }
                panel = panel.child(
                    crate::ui::action_button("elicitation-submit")
                        .small()
                        .primary()
                        .label(crate::i18n::tr!("action-submit"))
                        .disabled(self.submitting)
                        .on_click(cx.listener(|this, _, _, cx| this.answer("accept", cx))),
                );
            }
            Mode::Url(url) => {
                let url = url.clone();
                panel = panel
                    .child(crate::i18n::tr!(
                        "elicitation-host",
                        host = (url.host).to_string()
                    ))
                    .child(
                        div()
                            .id("elicitation-url")
                            .test_support()
                            .text_size(crate::ui::SMALL)
                            .child(url.url.clone()),
                    )
                    .child(
                        crate::ui::action_button("elicitation-open-url")
                            .primary()
                            .label(crate::i18n::tr!("action-open-link"))
                            .disabled(self.submitting)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.submitting {
                                    cx.open_url(&url.url);
                                    this.answer("accept", cx);
                                }
                            })),
                    );
            }
        }
        panel
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        crate::ui::action_button("elicitation-decline")
                            .label(crate::i18n::tr!("action-decline"))
                            .disabled(self.submitting)
                            .on_click(cx.listener(|this, _, _, cx| this.answer("decline", cx))),
                    )
                    .child(
                        crate::ui::action_button("elicitation-cancel")
                            .label(crate::i18n::tr!("action-cancel"))
                            .disabled(self.submitting)
                            .on_click(cx.listener(|this, _, _, cx| this.answer("cancel", cx))),
                    ),
            )
            .when(!self.message.is_empty(), |panel| {
                panel.child(self.message.clone())
            })
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
