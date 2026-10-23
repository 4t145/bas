use bas::model::{Role, ToolCallStatus};
use gpui_kit::ClickEvent;

use super::composer::{COMPOSER_CONTROL_INSET, COMPOSER_SIDE_PADDING};
use super::*;

pub(super) mod content;
mod resource;

pub(super) const CONTENT_WIDTH: f32 = 840.;
pub(super) const THREAD_BODY_PADDING: f32 = 12.;
const INITIALIZATION_MESSAGES_MAX_HEIGHT: f32 = 160.;
const APPROVAL_PANEL_HEIGHT: f32 = 240.;

pub(super) fn content_panel() -> Div {
    el! { .div { w_full; min_w_0; max_w: CONTENT_WIDTH.px; mx_auto; } }
}

impl Workspace {
    fn work_status(&self) -> String {
        if let Some(conversation) = &self.selected.timeline.conversation {
            if matches!(
                conversation.foreground_state,
                Some(bas::acp_v2::StateUpdate::RequiresAction(_))
            ) {
                return tr!("status-agent-confirming").into();
            }
            for item in conversation.items.iter().rev() {
                match item {
                    Item::Message {
                        message,
                        streaming: true,
                    } if message.role == Role::Thought => {
                        return tr!("status-thinking").into();
                    }
                    Item::Message {
                        message,
                        streaming: true,
                    } if message.role == Role::Agent => {
                        return tr!("status-responding").into();
                    }
                    Item::Tool(tool)
                        if matches!(
                            tool.status,
                            ToolCallStatus::Pending | ToolCallStatus::InProgress
                        ) =>
                    {
                        return tr!("status-tool", tool = (tool.title).to_string());
                    }
                    _ => {}
                }
            }
        }
        tr!("status-thinking").into()
    }

    pub(super) fn render_chat(&self, cx: &mut Context<Self>) -> AnyElement {
        el! {
            .div {
                # "thread-body";
                test_support;
                flex;
                flex_col;
                flex_1;
                min_h_0;
                w_full;
                @if (!self.window.compact)
                    pl: THREAD_BODY_PADDING.px;
                    pr: $(turn_navigation::RAIL_WIDTH + THREAD_BODY_PADDING).px;
                @end;
            } [
                ..$self.render_notices(cx),
                @if (!self.composer_fills_thread())
                    $self.render_timeline(cx),
                @end,
                @if (self.composer_fills_thread())
                    ..$self.render_initialization_messages(cx),
                @end,
                .content_panel {
                    # "thread-composition"; test_support;
                    flex; flex_col; flex_shrink_0;
                    @if (self.composer_fills_thread()) flex_1; min_h_0; @end;
                    @if (self.window.compact) px: COMPOSER_SIDE_PADDING.px; @end;
                } [
                    .div {
                        w_full;
                        # "conversation-status";
                        test_support;
                        flex;
                        items_center;
                        gap_2;
                        pl: COMPOSER_CONTROL_INSET.px;
                        py_1;
                    } [
                        $self.agent_button(cx),
                        ..$self.render_plan_indicator(cx),
                        $self.session_indicator("conversation-connection", cx),
                        @if (let Some(label) = self.activity_label())
                            (label, cx).caption { flex_1; min_w_0; truncate; },
                        @end,
                        .div { flex_1; },
                        @if (self.selected.id.is_some() && !self.composer_fills_thread())
                            $self.render_conversation_navigation(cx),
                        @end,
                    ],
                    $self.render_composer(cx),
                ],
            ]
        }
        .into_any_element()
    }

    pub(super) fn render_conversation_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        el! {
            .div {
                # "conversation-navigation";
                test_support;
                flex;
                items_center;
                gap_1;
            } [
                ("scroll-to-top", IconName::ArrowUp, cx).message_actions::action_button {
                    accessibility_label: tr!("conversation-top");
                    hoverable_tooltip: $shortcuts::tooltip(tr!("conversation-top"), shortcuts::ScrollConversationToTop);
                    on_click: (cx, Self::on_chat_top).Context::listener;
                },
                ("scroll-to-bottom", IconName::ArrowDown, cx).message_actions::action_button {
                    accessibility_label: tr!("conversation-latest");
                    hoverable_tooltip: $shortcuts::tooltip(tr!("conversation-latest"), ScrollConversationToEnd);
                    on_click: (cx, Self::on_chat_bottom).Context::listener;
                },
            ]
        }
        .into_any_element()
    }

    fn on_chat_top(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_conversation_to_top(cx);
    }

    fn on_chat_bottom(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_conversation_to_end(cx);
    }

    fn render_initialization_messages(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let conversation = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .filter(|state| !state.rows.is_empty())?;
        Some(
            el! {
                .content_panel {
                    # "agent-initialization-messages";
                    test_support;
                    flex_shrink_0;
                    max_h: INITIALIZATION_MESSAGES_MAX_HEIGHT.px;
                    overflow_y_scroll;
                    border_b_1;
                    border_color: $cx.theme().border;
                } [
                    (tr!("conversation-startup"), cx).caption { px_3; pt_2; },
                    ..$((0..conversation.rows.len()).map(|index| self.render_item(index, cx))),
                ]
            }
            .into_any_element(),
        )
    }

    pub(super) fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.awaiting_connection() {
            return self.render_connection_transition(cx);
        }
        let awaiting_history =
            self.selected.awaiting_history() && self.selected.authentication.panel.is_none();
        let empty = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .is_none_or(|conversation| conversation.items.is_empty());
        el! {
            .div {
                # "conversation-timeline";
                test_support;
                flex;
                flex_col;
                flex_1;
                min_h_0;
                size_full;
            } [
                @if (awaiting_history)
                    $self.render_history_transition(cx),
                @end,
                @if (empty && !awaiting_history && self.selected.authentication.panel.is_none())
                    $self.render_empty_timeline(cx),
                @end,
                @if (!empty)
                    $self.render_timeline_list(cx),
                @end,
                @if (!self.selected.approval.panel.is_empty())
                    $self.render_timeline_approval(cx),
                @end,
                ..$self.render_queue_panel(cx),
                @if (let Some(panel) = self.selected.authentication.panel.as_ref())
                    .content_panel {
                        # "thread-authentication";
                        test_support;
                        p_4; my_3; flex_shrink_0; min_w_0;
                        @if (empty) my_auto; @end;
                        border_1; rounded_md; border_color: $cx.theme().border;
                        bg: $cx.theme().muted.opacity(0.2);
                        max_h: 360.0.px;
                        overflow_y_scroll;
                    } [~$(panel.entity)],
                @end,
                @if (!self.selected.elicitations.is_empty())
                    .div {
                        # "thread-elicitations";
                        test_support;
                        @if (self.window.compact) px_3; @end;
                        py_2;
                        max_h: 360.0.px;
                        overflow_y_scroll;
                    } [
                        .content_panel [
                            ..$(self.selected.elicitations.values()) { map: panel => ~$(panel.entity) },
                        ],
                    ],
                @end,
            ]
        }.into_any_element()
    }

    fn render_history_transition(&self, cx: &mut Context<Self>) -> AnyElement {
        let (label, loading) = match &self.selected.stream {
            StreamState::Loading | StreamState::Live => (tr!("status-loading"), true),
            StreamState::Reconnecting(_) => (tr!("status-reconnecting"), true),
            StreamState::Unavailable(_) => (tr!("status-session-unavailable"), false),
            StreamState::Closed => (tr!("status-session-ended"), false),
        };
        let failure = match &self.selected.stream {
            StreamState::Unavailable(error) | StreamState::Reconnecting(error) => {
                Some(error.presentation())
            }
            _ => None,
        };
        let retry = !loading && self.selected.id.is_some();
        el! { .content_panel {
            # "conversation-history-transition"; test_support;
            role: gpui_kit::Role::Status; aria_label: label;
            flex_1; min_h_0; flex; flex_col; items_center; justify_center; gap_3; p_5;
            text_size: ui::SMALL; text_color: $cx.theme().muted_foreground;
        } [
            @if (loading)
                ("conversation-history-loading", IconName::LoaderCircle, $cx.theme().muted_foreground, true).status::activity_icon,
            @end,
            label,
            @if (let Some(message) = failure)
                ("history-load-error", message, $cx.theme().danger).ui::feedback,
            @end,
            @if (retry)
                "history-load-retry".ui::action_button {
                    label: $self.history_retry_label();
                    disabled: $self.history_retry_disabled();
                    on_click: $cx.listener(|this, _, _, cx| this.retry_history(cx));
                },
            @end,
        ] }.into_any_element()
    }

    fn render_empty_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.selected.creating {
            tr!("status-creating")
        } else if self.selected.waiting_for_agent_selection {
            tr!("conversation-choose-agent")
        } else if self.selected.id.is_some() || self.window.compact {
            tr!("conversation-empty")
        } else {
            tr!("conversation-start")
        };
        el! {
            .div {
                # "empty-conversation-message"; test_support;
                w_full;
                flex_1;
                min_h_0;
                flex;
                flex_col;
                justify_center;
                items_center;
                gap_2;
                p_5;
            } [
                (label, cx).caption,
                @if (!self.window.compact && self.selected.id.is_none())
                    ($(navigation::groups::path_label(self.navigation.selected_directory.as_deref())), cx).caption,
                @end,
            ]
        }
        .into_any_element()
    }

    fn render_timeline_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        el! {
            .div {
                relative; flex_1; min_h_0; w_full;
                on_scroll_wheel: (cx, Self::on_navigation_wheel).Context::listener;
            } [
                (~$(self.selected.timeline.list), move (index, _, cx) => $view.update(cx, |this, cx| this.render_item(index, cx))).list { size_full; },
                .div {
                    absolute; inset_0;
                    @if (self.window.compact)
                        pl: THREAD_BODY_PADDING.px;
                        pr: $(turn_navigation::RAIL_WIDTH + THREAD_BODY_PADDING).px;
                    @end;
                } [
                    .content_panel {
                        # "timeline-navigation-anchor"; test_support; relative; h_full;
                    } [$self.render_turn_navigation(cx)],
                ],
                .div {
                    # "timeline-scrollbar"; test_support;
                    absolute; top_0; bottom_0; w: THREAD_BODY_PADDING.px;
                    right: $(if self.window.compact { 0. } else { -turn_navigation::RAIL_WIDTH - THREAD_BODY_PADDING }).px;
                } [
                    $gpui_kit::base::Scrollbar::vertical(&self.selected.timeline.list)
                        .viewport_from_layout()
                        .mode(gpui_kit::base::ScrollbarMode::Always),
                ],
                ..$self.render_older_history(cx),
                ..$self.render_history_location_status(cx),
            ]
        }
        .into_any_element()
    }

    fn render_timeline_approval(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.selected.id;
        let epoch = self.selected.epoch;
        el! {
            .div {
                # "approval-panel";
                test_support;
                @if (self.window.compact) px_3; @end;
                py_2;
                flex_shrink_0;
                max_h: APPROVAL_PANEL_HEIGHT.px;
                overflow_y_scroll;
            } [
                .content_panel [
                    $self.selected.approval.panel.render(cx, move |this, request_id, outcome, _, cx| {
                        if let Some(session) = session {
                            this.respond_permission(session, epoch, request_id, outcome, cx);
                        }
                    }),
                ],
            ]
        }.into_any_element()
    }

    pub(super) fn stream_label(&self) -> String {
        if self.composer.paste_pending {
            return tr!("status-pasting").into();
        }
        if !self.selected.approval.panel.is_empty() {
            return tr!("status-approval").into();
        }
        if self.selected.approval.error.is_some() {
            return tr!("status-approval-unavailable").into();
        }
        if !self.selected.elicitations.is_empty() {
            return tr!("status-input").into();
        }
        if !self.backend.connected {
            return if self.backend.error.is_some() {
                tr!("status-reconnecting").into()
            } else {
                tr!("status-connecting").into()
            };
        }
        if self.selected.id.is_none() {
            return if self.selected.creating {
                tr!("status-creating").into()
            } else if self.selected.waiting_for_agent_selection {
                tr!("status-select-agent").into()
            } else if self.selected.authentication.required.is_some() {
                tr!("status-auth").into()
            } else if self.backend.agents.is_empty() {
                tr!("status-no-agents").into()
            } else {
                tr!(
                    "composer-shortcuts-custom",
                    send = self.composer_send_shortcut(),
                    newline = self.composer_newline_shortcut()
                )
            };
        }
        if self
            .selected
            .id
            .is_some_and(|id| !self.sessions.healthy(id))
        {
            return tr!("status-failed").into();
        }
        if self
            .selected
            .id
            .is_some_and(|id| self.sessions.pending.configuring.contains(&id))
        {
            return tr!("status-applying").into();
        }
        if self
            .selected
            .id
            .and_then(|id| self.sessions.outboxes.get(&id))
            .is_some_and(|outbox| outbox.unknown().is_some())
        {
            return tr!("status-submission-unknown").into();
        }
        if self.selected.id.is_some_and(|id| self.sending(id)) {
            return tr!("status-sending").into();
        }
        match &self.selected.stream {
            StreamState::Loading => tr!("status-loading").into(),
            StreamState::Live if self.working() => self.work_status(),
            StreamState::Live => tr!("status-synced").into(),
            StreamState::Reconnecting(_) => tr!("status-reconnecting").into(),
            StreamState::Unavailable(_) => tr!("status-session-unavailable").into(),
            StreamState::Closed => tr!("status-session-ended").into(),
        }
    }

    fn render_item(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(conversation) = self.selected.timeline.conversation.as_ref() else {
            return el! { .div }.into_any_element();
        };
        let Some(row) = conversation.rows.get(index) else {
            return el! { .div }.into_any_element();
        };
        let items = &conversation.items[row.items.clone()];
        let item = &items[0];
        let grouped = row.activity && items.len() > 1;
        let id = if grouped {
            format!("activity-{}", item.id())
        } else {
            item.id()
        };
        let content = if grouped {
            self.render_activity(&id, items, index, cx)
        } else {
            self.render_entry(item, index, cx)
        };
        let panel_id = SharedString::from(format!("message-content-{id}"));
        el! {
            .div {
                # id.SharedString::from;
                test_support;
                @if (self.window.compact)
                    pl: THREAD_BODY_PADDING.px;
                    pr: $(turn_navigation::RAIL_WIDTH + THREAD_BODY_PADDING).px;
                @end;
                py_1; w_full; min_w_0;
                text_size: $theme::content_font_size(cx);
            } [
                .content_panel { # panel_id; test_support; } [
                    content,
                    ..$self.render_message_actions(
                            items
                                .last()
                                .expect("timeline rows contain at least one item"),
                            cx,
                    ),
                ],
            ]
        }
        .into_any_element()
    }
}
