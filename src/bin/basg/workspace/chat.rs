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
    gds_uim::el! { #div { w_full; min_w_0; max_w: $px(CONTENT_WIDTH); mx_auto; } }
}

impl Workspace {
    fn work_status(&self) -> String {
        if let Some(conversation) = &self.selected.timeline.conversation {
            if matches!(
                conversation.foreground_state,
                Some(bas::acp_v2::StateUpdate::RequiresAction(_))
            ) {
                return crate::i18n::tr!("status-agent-confirming").into();
            }
            for item in conversation.items.iter().rev() {
                match item {
                    Item::Message {
                        message,
                        streaming: true,
                    } if message.role == Role::Thought => {
                        return crate::i18n::tr!("status-thinking").into();
                    }
                    Item::Message {
                        message,
                        streaming: true,
                    } if message.role == Role::Agent => {
                        return crate::i18n::tr!("status-responding").into();
                    }
                    Item::Tool(tool)
                        if matches!(
                            tool.status,
                            ToolCallStatus::Pending | ToolCallStatus::InProgress
                        ) =>
                    {
                        return crate::i18n::tr!("status-tool", tool = (tool.title).to_string());
                    }
                    _ => {}
                }
            }
        }
        crate::i18n::tr!("status-thinking").into()
    }

    pub(super) fn render_chat(&self, cx: &mut Context<Self>) -> AnyElement {
        gds_uim::el! {
            #div {
                id: "thread-body";
                test_support;
                flex;
                flex_col;
                flex_1;
                min_h_0;
                w_full;
                @$(!self.window.compact) => {
                    pl: $px(THREAD_BODY_PADDING);
                    pr: $px(turn_navigation::RAIL_WIDTH + THREAD_BODY_PADDING);
                };
            } [
                ..$self.render_notices(cx),
                ..$((!self.composer_fills_thread()).then(|| self.render_timeline(cx))),
                ..$self.composer_fills_thread().then(|| self.render_initialization_messages(cx)).flatten(),
                #content_panel() {
                    id: "thread-composition"; test_support;
                    flex; flex_col; flex_shrink_0;
                    @$self.composer_fills_thread() => { flex_1; min_h_0; };
                    @$self.window.compact => { px: $px(COMPOSER_SIDE_PADDING); };
                } [
                    #div {
                        w_full;
                        id: "conversation-status";
                        test_support;
                        flex;
                        items_center;
                        gap_2;
                        pl: $px(COMPOSER_CONTROL_INSET);
                        py_1;
                    } [
                        $self.agent_button(cx),
                        ..$self.render_plan_indicator(cx),
                        $self.session_indicator("conversation-connection", cx),
                        ..$self.activity_label().map(|label| gds_uim::el! {
                            $caption(label, cx) { flex_1; min_w_0; truncate; }
                        }),
                        #div { flex_1; },
                        ..$((self.selected.id.is_some() && !self.composer_fills_thread())
                            .then(|| self.render_conversation_navigation(cx))),
                    ],
                    $self.render_composer(cx),
                ],
            ]
        }.into_any_element()
    }

    pub(super) fn render_conversation_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        gds_uim::el! {
            #div {
                id: "conversation-navigation";
                test_support;
                flex;
                items_center;
                gap_1;
            } [
                $message_actions::action_button("scroll-to-top", IconName::ArrowUp, cx) {
                    accessibility_label: $crate::i18n::tr!("conversation-top");
                    hoverable_tooltip: $shortcuts::tooltip(crate::i18n::tr!("conversation-top"), shortcuts::ScrollConversationToTop);
                    on_click: $cx.listener(Self::on_chat_top);
                },
                $message_actions::action_button("scroll-to-bottom", IconName::ArrowDown, cx) {
                    accessibility_label: $crate::i18n::tr!("conversation-latest");
                    hoverable_tooltip: $shortcuts::tooltip(crate::i18n::tr!("conversation-latest"), ScrollConversationToEnd);
                    on_click: $cx.listener(Self::on_chat_bottom);
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
            gds_uim::el! {
                #content_panel() {
                    id: "agent-initialization-messages";
                    test_support;
                    flex_shrink_0;
                    max_h: $px(INITIALIZATION_MESSAGES_MAX_HEIGHT);
                    overflow_y_scroll;
                    border_b_1;
                    border_color: $cx.theme().border;
                } [
                    $caption(crate::i18n::tr!("conversation-startup"), cx) { px_3; pt_2; },
                    ..$((0..conversation.rows.len()).map(|index| self.render_item(index, cx))),
                ]
            }
            .into_any_element(),
        )
    }

    pub(super) fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let awaiting_snapshot =
            self.selected.awaiting_snapshot() && self.selected.authentication.panel.is_none();
        let empty = self
            .selected
            .timeline
            .conversation
            .as_ref()
            .is_none_or(|conversation| conversation.items.is_empty());
        gds_uim::el! {
            #div {
                id: "conversation-timeline";
                test_support;
                flex;
                flex_col;
                flex_1;
                min_h_0;
                size_full;
            } [
                ..$awaiting_snapshot.then(|| self.render_history_transition(cx)),
                ..$((empty && !awaiting_snapshot && self.selected.authentication.panel.is_none()).then(|| self.render_empty_timeline(cx))),
                ..$((!empty).then(|| self.render_timeline_list(cx))),
                ..$((!self.selected.approval.panel.is_empty()).then(|| self.render_timeline_approval(cx))),
                ..$self.render_queue_panel(cx),
                ..$self.selected.authentication.panel.as_ref().map(|panel| {
                    gds_uim::el! {
                        #content_panel() {
                            id: "thread-authentication";
                            test_support;
                            p_4; my_3; flex_shrink_0; min_w_0;
                            @empty => { my_auto; };
                            border_1; rounded_md; border_color: $cx.theme().border;
                            bg: $cx.theme().muted.opacity(0.2);
                            max_h: $px(360.);
                            overflow_y_scroll;
                        } [$panel.entity.clone()]
                    }
                }),
                ..$((!self.selected.elicitations.is_empty()).then(|| {
                    gds_uim::el! {
                        #div {
                            id: "thread-elicitations";
                            test_support;
                            @$self.window.compact => { px_3; };
                            py_2;
                            max_h: $px(360.);
                            overflow_y_scroll;
                        } [
                            $content_panel() [
                                ..$self.selected.elicitations.values().map(|panel| panel.entity.clone())
                            ],
                        ]
                    }
                })),
            ]
        }.into_any_element()
    }

    fn render_history_transition(&self, cx: &mut Context<Self>) -> AnyElement {
        let (label, loading) = match &self.selected.stream {
            StreamState::Loading | StreamState::Live => (crate::i18n::tr!("status-loading"), true),
            StreamState::Reconnecting(_) => (crate::i18n::tr!("status-reconnecting"), true),
            StreamState::Unavailable(_) => (crate::i18n::tr!("status-session-unavailable"), false),
            StreamState::Closed => (crate::i18n::tr!("status-session-ended"), false),
        };
        let failure = match &self.selected.stream {
            StreamState::Unavailable(error) | StreamState::Reconnecting(error) => {
                Some(error.presentation())
            }
            _ => None,
        };
        let progress = (loading && self.selected.history_loaded > 0).then(|| {
            crate::i18n::tr!(
                "session-history-progress",
                count = self.selected.history_loaded
            )
        });
        let retry = !loading && self.selected.id.is_some();
        gds_uim::el! { #content_panel() {
            id: "conversation-history-transition"; test_support;
            role: gpui_kit::Role::Status; aria_label: label;
            flex_1; min_h_0; flex; flex_col; items_center; justify_center; gap_3; p_5;
            text_size: crate::ui::SMALL; text_color: $cx.theme().muted_foreground;
        } [
            ..$loading.then(|| status::activity_icon(
                "conversation-history-loading", IconName::LoaderCircle, cx.theme().muted_foreground, true
            )),
            $label,
            ..progress,
            ..$failure.map(|message| crate::ui::feedback(
                "history-load-error", message, cx.theme().danger
            )),
            ..$retry.then(|| gds_uim::el! {
                #crate::ui::action_button("history-load-retry") {
                    label: $crate::i18n::tr!("session-history-retry");
                    on_click: $cx.listener(|this, _, _, cx| this.retry_history(cx));
                }
            }),
        ] }.into_any_element()
    }

    fn render_empty_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.selected.creating {
            crate::i18n::tr!("status-creating")
        } else if self.selected.waiting_for_agent_selection {
            crate::i18n::tr!("conversation-choose-agent")
        } else if self.selected.id.is_some() || self.window.compact {
            crate::i18n::tr!("conversation-empty")
        } else {
            crate::i18n::tr!("conversation-start")
        };
        gds_uim::el! {
            #div {
                id: "empty-conversation-message"; test_support;
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
                $caption(label, cx),
                ..$((!self.window.compact && self.selected.id.is_none()).then(|| {
                    caption(navigation::groups::path_label(self.navigation.selected_directory.as_deref()), cx)
                })),
            ]
        }
        .into_any_element()
    }

    fn render_timeline_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        gds_uim::el! {
            #div { relative; flex_1; min_h_0; w_full; } [
                $list(self.selected.timeline.list.clone(), move |index, _, cx| {
                    view.update(cx, |this, cx| this.render_item(index, cx))
                }) { size_full; },
                #div {
                    absolute; inset_0;
                    @$self.window.compact => {
                        pl: THREAD_BODY_PADDING.px;
                        pr: $(turn_navigation::RAIL_WIDTH + THREAD_BODY_PADDING).px;
                    };
                } [
                    #content_panel() {
                        id: "timeline-navigation-anchor"; test_support; relative; h_full;
                    } [$self.render_turn_navigation(cx)],
                ],
                #div {
                    id: "timeline-scrollbar"; test_support;
                    absolute; top_0; bottom_0; w: THREAD_BODY_PADDING.px;
                    right: $(if self.window.compact { 0. } else { -turn_navigation::RAIL_WIDTH - THREAD_BODY_PADDING }).px;
                } [
                    $gpui_kit::base::Scrollbar::vertical(&self.selected.timeline.list)
                        .viewport_from_layout()
                        .mode(gpui_kit::base::ScrollbarMode::Always),
                ],
            ]
        }
        .into_any_element()
    }

    fn render_timeline_approval(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.selected.id;
        let epoch = self.selected.epoch;
        gds_uim::el! {
            #div {
                id: "approval-panel";
                test_support;
                @$self.window.compact => { px_3; };
                py_2;
                flex_shrink_0;
                max_h: $px(APPROVAL_PANEL_HEIGHT);
                overflow_y_scroll;
            } [
                $content_panel() [
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
            return crate::i18n::tr!("status-pasting").into();
        }
        if !self.selected.approval.panel.is_empty() {
            return crate::i18n::tr!("status-approval").into();
        }
        if self.selected.approval.error.is_some() {
            return crate::i18n::tr!("status-approval-unavailable").into();
        }
        if !self.selected.elicitations.is_empty() {
            return crate::i18n::tr!("status-input").into();
        }
        if !self.backend.connected {
            return if self.backend.error.is_some() {
                crate::i18n::tr!("status-reconnecting").into()
            } else {
                crate::i18n::tr!("status-connecting").into()
            };
        }
        if self.selected.id.is_none() {
            return if self.selected.creating {
                crate::i18n::tr!("status-creating").into()
            } else if self.selected.waiting_for_agent_selection {
                crate::i18n::tr!("status-select-agent").into()
            } else if self.selected.authentication.required.is_some() {
                crate::i18n::tr!("status-auth").into()
            } else if self.backend.agents.is_empty() {
                crate::i18n::tr!("status-no-agents").into()
            } else {
                crate::i18n::tr!(
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
            return crate::i18n::tr!("status-failed").into();
        }
        if self
            .selected
            .id
            .is_some_and(|id| self.sessions.pending.configuring.contains(&id))
        {
            return crate::i18n::tr!("status-applying").into();
        }
        if self
            .selected
            .id
            .and_then(|id| self.sessions.outboxes.get(&id))
            .is_some_and(|outbox| outbox.unknown().is_some())
        {
            return crate::i18n::tr!("status-submission-unknown").into();
        }
        if self.selected.id.is_some_and(|id| self.sending(id)) {
            return crate::i18n::tr!("status-sending").into();
        }
        match &self.selected.stream {
            StreamState::Loading => crate::i18n::tr!("status-loading").into(),
            StreamState::Live if self.working() => self.work_status(),
            StreamState::Live => crate::i18n::tr!("status-synced").into(),
            StreamState::Reconnecting(_) => crate::i18n::tr!("status-reconnecting").into(),
            StreamState::Unavailable(_) => crate::i18n::tr!("status-session-unavailable").into(),
            StreamState::Closed => crate::i18n::tr!("status-session-ended").into(),
        }
    }

    fn render_item(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(conversation) = self.selected.timeline.conversation.as_ref() else {
            return gds_uim::el! { #div }.into_any_element();
        };
        let Some(row) = conversation.rows.get(index) else {
            return gds_uim::el! { #div }.into_any_element();
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
        gds_uim::el! {
            #div {
                id: $SharedString::from(id);
                test_support;
                @$self.window.compact => {
                    pl: $px(THREAD_BODY_PADDING);
                    pr: $px(turn_navigation::RAIL_WIDTH + THREAD_BODY_PADDING);
                };
                py_1; w_full; min_w_0;
                text_size: $theme::content_font_size(cx);
            } [
                $content_panel() { id: $panel_id; test_support; } [
                    $content,
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
