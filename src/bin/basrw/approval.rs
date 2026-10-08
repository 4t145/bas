//! Live tool approval cards use the choices supplied by the ACP agent.

use std::collections::HashSet;

use basr::{acp, model::PendingPermission};
use gpui_kit::{
    AnyElement, Context, SharedString, Window,
    component::{ActiveTheme, Disableable, Sizable, button::*},
    div,
    prelude::*,
    px,
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

const DETAILS_HEIGHT: f32 = 180.;

#[derive(Default)]
pub(super) struct Panel {
    pending: Vec<PendingPermission>,
    in_flight: HashSet<String>,
    error: Option<String>,
}

impl Panel {
    pub fn clear(&mut self) {
        self.pending.clear();
        self.in_flight.clear();
        self.error = None;
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn replace(&mut self, pending: Vec<PendingPermission>) {
        self.in_flight
            .retain(|id| pending.iter().any(|request| &request.request_id == id));
        self.pending = pending;
        if self.pending.is_empty() {
            self.error = None;
        }
    }

    pub fn submitting(&mut self, request_id: &str) {
        self.in_flight.insert(request_id.to_owned());
        self.error = None;
    }

    pub fn responded(&mut self, request_id: &str, result: Result<(), String>) {
        // Success stays disabled until the authoritative pending snapshot arrives.
        if let Err(error) = result {
            self.in_flight.remove(request_id);
            if self
                .pending
                .iter()
                .any(|request| request.request_id == request_id)
            {
                self.error = Some(error);
            }
        }
    }

    pub fn render<Owner: 'static>(
        &self,
        cx: &mut Context<Owner>,
        decide: impl Fn(
            &mut Owner,
            String,
            acp::RequestPermissionOutcome,
            &mut Window,
            &mut Context<Owner>,
        ) + Clone
        + 'static,
    ) -> AnyElement {
        let mut panel = div().flex().flex_col().gap_2().w_full();
        for permission in &self.pending {
            let request_id = &permission.request_id;
            let disabled = self.in_flight.contains(request_id);
            let fields = &permission.request.tool_call.fields;
            let title = fields
                .title
                .clone()
                .unwrap_or_else(|| permission.request.tool_call.tool_call_id.to_string());
            let mut choices = div().flex().flex_wrap().gap_2();
            for option in &permission.request.options {
                let id = request_id.clone();
                let outcome = acp::RequestPermissionOutcome::Selected(
                    acp::SelectedPermissionOutcome::new(option.option_id.clone()),
                );
                let handler = decide.clone();
                let allow = matches!(
                    option.kind,
                    acp::PermissionOptionKind::AllowOnce | acp::PermissionOptionKind::AllowAlways
                );
                choices = choices.child(
                    Button::new(SharedString::from(format!(
                        "permission-{id}-{}",
                        option.option_id
                    )))
                    .small()
                    .label(option.name.clone())
                    .when(allow, |button| button.primary())
                    .disabled(disabled)
                    .on_click(cx.listener(move |owner, _, window, cx| {
                        handler(owner, id.clone(), outcome.clone(), window, cx);
                    })),
                );
            }
            let id = request_id.clone();
            let handler = decide.clone();
            choices = choices.child(
                Button::new(SharedString::from(format!("permission-cancel-{id}")))
                    .small()
                    .ghost()
                    .label("取消请求")
                    .disabled(disabled)
                    .on_click(cx.listener(move |owner, _, window, cx| {
                        handler(
                            owner,
                            id.clone(),
                            acp::RequestPermissionOutcome::Cancelled,
                            window,
                            cx,
                        );
                    })),
            );
            let details = details(&permission.request);
            panel = panel.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().warning)
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().warning)
                            .child("等待工具审批"),
                    )
                    .child(div().text_sm().child(title))
                    .when(!details.is_empty(), |card| {
                        card.child(
                            div()
                                .id(SharedString::from(format!(
                                    "permission-details-{request_id}"
                                )))
                                .max_h(px(DETAILS_HEIGHT))
                                .overflow_y_scroll()
                                .text_xs()
                                .child(details),
                        )
                    })
                    .child(choices)
                    .when(disabled, |card| {
                        card.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("正在提交审批…"),
                        )
                    }),
            );
        }
        panel
            .children(self.error.iter().map(|error| {
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
            }))
            .into_any_element()
    }
}

fn details(request: &acp::RequestPermissionRequest) -> String {
    let fields = &request.tool_call.fields;
    let mut parts = Vec::new();
    for location in fields.locations.iter().flatten() {
        parts.push(location.line.map_or_else(
            || location.path.display().to_string(),
            |line| format!("{}:{line}", location.path.display()),
        ));
    }
    if let Some(input) = &fields.raw_input {
        parts.push(serde_json::to_string_pretty(input).expect("JSON value serializes"));
    }
    for content in fields.content.iter().flatten() {
        match content {
            acp::ToolCallContent::Content(content) => {
                if let acp::ContentBlock::Text(text) = &content.content {
                    parts.push(text.text.clone());
                }
            }
            acp::ToolCallContent::Diff(diff) => {
                parts.push(format!(
                    "{}\n修改后：\n{}",
                    diff.path.display(),
                    diff.new_text
                ));
            }
            _ => {}
        }
    }
    parts.join("\n\n")
}
