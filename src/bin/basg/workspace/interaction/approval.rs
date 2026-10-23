//! Live tool approval cards use the choices supplied by the ACP agent.

use std::collections::{HashMap, HashSet};

use crate::{i18n::tr, ui};
use bas::{acp, model::PendingPermission};
use gds_uim::el;
use gpui_kit::{
    AnyElement, Context, SharedString, Window,
    component::{ActiveTheme, Disableable, button::*},
    div,
    prelude::*,
    px,
};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

const DETAILS_HEIGHT: f32 = 180.;

#[derive(Default)]
pub(in crate::workspace) struct Panel {
    pending: Vec<PendingPermission>,
    in_flight: HashSet<String>,
    errors: HashMap<String, String>,
}

impl Panel {
    pub(in crate::workspace) fn clear(&mut self) {
        self.pending.clear();
        self.in_flight.clear();
        self.errors.clear();
    }

    pub(in crate::workspace) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(in crate::workspace) fn replace(&mut self, pending: Vec<PendingPermission>) {
        self.in_flight
            .retain(|id| pending.iter().any(|request| &request.request_id == id));
        self.errors
            .retain(|id, _| pending.iter().any(|request| &request.request_id == id));
        self.pending = pending;
    }

    pub(in crate::workspace) fn submitting(&mut self, request_id: &str) -> bool {
        if !self
            .pending
            .iter()
            .any(|request| request.request_id == request_id)
            || !self.in_flight.insert(request_id.to_owned())
        {
            return false;
        }
        self.errors.remove(request_id);
        true
    }

    pub(in crate::workspace) fn responded(&mut self, request_id: &str, result: Result<(), String>) {
        // Success stays disabled until the authoritative pending snapshot arrives.
        if let Err(error) = result {
            self.in_flight.remove(request_id);
            if self
                .pending
                .iter()
                .any(|request| request.request_id == request_id)
            {
                self.errors.insert(request_id.to_owned(), error);
            }
        }
    }

    pub(in crate::workspace) fn render<Owner: 'static>(
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
        let mut cards = Vec::new();
        for permission in &self.pending {
            let request_id = &permission.request_id;
            let disabled = self.in_flight.contains(request_id);
            let fields = &permission.request.tool_call.fields;
            let title = fields
                .title
                .clone()
                .unwrap_or_else(|| permission.request.tool_call.tool_call_id.to_string());
            let mut choices = Vec::new();
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
                choices.push(el! { format!(
                    "permission-{id}-{}",
                    option.option_id
                ).SharedString::from.ui::action_button
                {
                    label: ~$(option.name);
                    @if (allow) primary; @end;
                    disabled: disabled;
                    on_click: $cx.listener(move |owner, _, window, cx| {
                        handler(owner, id.clone(), outcome.clone(), window, cx);
                    });
                } });
            }
            let id = request_id.clone();
            let handler = decide.clone();
            choices.push(
                el! { format!("permission-cancel-{id}").SharedString::from.ui::action_button
                    {
                        ghost; label: tr!("approval-cancel");
                        disabled: disabled;
                        on_click: $cx.listener(move |owner, _, window, cx| {
                            handler(owner, id.clone(), acp::RequestPermissionOutcome::Cancelled, window, cx);
                        });
                    } },
            );
            let details = details(&permission.request);
            cards.push(el! {
                .div {
                    w_full; min_w_0; flex; flex_col; gap_2; p_3;
                    rounded_lg; border_1; border_color: $cx.theme().warning;
                } [
                    (format!("approval-{request_id}-waiting"), tr!("approval-waiting"), $cx.theme().warning).ui::feedback,
                    .div { text_size: ui::SMALL; } [title],
                    @if (!details.is_empty())
                        .div {
                            # format!("permission-details-{request_id}").SharedString::from;
                            max_h: DETAILS_HEIGHT.px; overflow_y_scroll; text_size: ui::SMALL;
                        } [details],
                    @end,
                    .div { flex; flex_wrap; gap_2; } [..choices],
                    @if (disabled)
                        (
                            format!("approval-{request_id}-submitting"),
                            tr!("approval-submitting"),
                            $cx.theme().muted_foreground,
                        ).ui::feedback,
                    @end,
                    @if (let Some(error) = self.errors.get(request_id))
                        (
                            format!("approval-{request_id}-error"),
                            ~error,
                            $cx.theme().danger,
                        ).ui::feedback,
                    @end,
                ]
            });
        }
        el! { .div { flex; flex_col; gap_2; w_full; } [..cards] }.into_any_element()
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
                parts.push(crate::i18n::tr!(
                    "diff-modified",
                    path = (diff.path.display()).to_string(),
                    content = (diff.new_text).to_string()
                ));
            }
            _ => {}
        }
    }
    parts.join("\n\n")
}
