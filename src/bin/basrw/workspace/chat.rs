use basr::acp;
use basr::model::{ContentBlock, Role, ToolCall, ToolCallStatus};

use super::*;

pub(super) const CONTENT_WIDTH: f32 = 840.;
pub(super) const THREAD_BODY_PADDING: f32 = 12.;
pub(super) const COMPOSER_SIDE_PADDING: f32 = 4.;
pub(super) const COMPOSER_TOOLBAR_HEIGHT: f32 = 24.;
pub(super) const COMPOSER_BOTTOM_PADDING: f32 = 2.;
pub(super) const COMPOSER_CONTROL_INSET: f32 = 4.;
const COMPOSER_TEXT_INSET: f32 =
    COMPOSER_CONTROL_INSET + (layout::ICON_BUTTON_SIZE - forms::TOOLBAR_AGENT_ICON_SIZE) / 2.;
const COMPOSER_CONTROL_GAP: f32 = 4.;
const INITIALIZATION_MESSAGES_MAX_HEIGHT: f32 = 160.;
const APPROVAL_PANEL_HEIGHT: f32 = 240.;
// The pinned editor reserves 10px each for its gutter and caret at the right edge.
const EDITOR_WRAP_INSET: f32 = 20.;

pub(super) fn content_panel() -> Div {
    div().w_full().max_w(px(CONTENT_WIDTH)).mx_auto()
}

impl Workspace {
    pub(super) fn composer_fills_thread(&self) -> bool {
        !self.compact
            && self.composer_expanded
            && !self.thread_has_user_prompt()
            && !self.working()
            && self.approvals.is_empty()
            && self.authentication_panel.is_none()
            && self.elicitation_panels.is_empty()
            && self.active.is_none_or(|id| {
                !self
                    .outboxes
                    .get(&id)
                    .is_some_and(outbox::SessionOutbox::pending)
            })
    }

    pub(super) fn measure_composer(&mut self, cx: &mut Context<Self>) {
        let input = self.composer.read(cx);
        let width = input
            .text_bounds()
            .map(|bounds| (bounds.size.width - px(EDITOR_WRAP_INSET)).max(px(1.)));
        let text = input.text().clone();
        let mut font = gpui_kit::font(cx.theme().font_family.clone());
        font.fallbacks = theme::font_style(cx).text.font_fallbacks;
        self.composer_rows =
            self.composer_layout
                .measure(&text, font, theme::content_font_size(cx), width, cx);
    }

    pub(super) fn composer_height(&self, cx: &App) -> f32 {
        let input = self.composer.read(cx);
        let rows = input_layout::visible_rows(
            self.composer_rows,
            input.text().lines_len(),
            self.compact,
            self.composer_expanded,
        );
        let height = rows as f32
            * (f32::from(theme::content_font_size(cx)) * theme::content_line_height(cx)).ceil()
            + f32::from(gpui_kit::component::Size::Medium.input_py()) * 2.;
        if self.composer_expanded && !self.compact {
            height.min(self.composer_available_height.get())
        } else {
            height
        }
    }

    fn work_status(&self) -> String {
        if let Some(conversation) = &self.conversation {
            if matches!(
                conversation.foreground_state,
                Some(basr::acp_v2::StateUpdate::RequiresAction(_))
            ) {
                return "等待 Agent 确认…".into();
            }
            for item in conversation.items.iter().rev() {
                match item {
                    Item::Message {
                        message,
                        streaming: true,
                    } if message.role == Role::Thought => return "思考中…".into(),
                    Item::Message {
                        message,
                        streaming: true,
                    } if message.role == Role::Agent => return "正在输出…".into(),
                    Item::Tool(tool)
                        if matches!(
                            tool.status,
                            ToolCallStatus::Pending | ToolCallStatus::InProgress
                        ) =>
                    {
                        return format!("调用工具 · {}", tool.title);
                    }
                    _ => {}
                }
            }
        }
        "思考中…".into()
    }

    pub(super) fn render_chat(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("thread-body")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .when(!self.compact, |area| {
                area.pl(px(THREAD_BODY_PADDING))
                    .pr(px(turn_navigation::RAIL_WIDTH))
            })
            .children(self.render_plan(cx))
            .children(self.render_notices(cx))
            .when(!self.composer_fills_thread(), |area| {
                area.child(self.render_timeline(cx))
            })
            .when(self.composer_fills_thread(), |area| {
                area.children(self.render_initialization_messages(cx))
            })
            .child(
                div()
                    .flex_shrink_0()
                    .when(self.compact, |area| area.px(px(COMPOSER_SIDE_PADDING)))
                    .child(
                        div()
                            .w_full()
                            .id("conversation-status")
                            .test_support()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pl(px(COMPOSER_CONTROL_INSET + 1.))
                            .py_1()
                            .child(self.agent_button(cx))
                            .child(self.connection_indicator("conversation-connection", cx))
                            .children(
                                self.activity_label()
                                    .map(|label| caption(label, cx).flex_1().min_w_0().truncate()),
                            )
                            .child(div().flex_1())
                            .when(
                                self.active.is_some() && !self.composer_fills_thread(),
                                |row| row.child(self.render_conversation_navigation(cx)),
                            ),
                    ),
            )
            .child(self.render_composer(cx))
            .into_any_element()
    }

    pub(super) fn render_conversation_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("conversation-navigation")
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .child(
                message_actions::action_button("scroll-to-top", IconName::ArrowUp, cx)
                    .accessibility_label("滚动到对话顶部")
                    .tooltip("滚动到对话顶部")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.list.scroll_to(gpui_kit::ListOffset::default());
                        cx.notify();
                    })),
            )
            .child(
                message_actions::action_button("scroll-to-bottom", IconName::ArrowDown, cx)
                    .accessibility_label("滚动到最新输出")
                    .tooltip("滚动到最新输出")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.scroll_conversation_to_end(cx);
                    })),
            )
            .into_any_element()
    }

    fn render_initialization_messages(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let conversation = self
            .conversation
            .as_ref()
            .filter(|state| !state.rows.is_empty())?;
        Some(
            div()
                .id("agent-initialization-messages")
                .test_support()
                .flex_shrink_0()
                .max_h(px(INITIALIZATION_MESSAGES_MAX_HEIGHT))
                .overflow_y_scroll()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(caption("初始化信息", cx).px_3().pt_2())
                .children((0..conversation.rows.len()).map(|index| self.render_item(index, cx)))
                .into_any_element(),
        )
    }

    pub(super) fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let empty = self
            .conversation
            .as_ref()
            .is_none_or(|conversation| conversation.items.is_empty());
        div()
            .id("conversation-timeline")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .size_full()
            .when(empty, |area| {
                area.child(
                    div()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .items_center()
                        .gap_2()
                        .p_5()
                        .child(caption(
                            if self.creating {
                                "正在创建 thread…"
                            } else if self.waiting_for_agent_selection {
                                "选择 Agent 后开始新 thread"
                            } else if self.active.is_some() {
                                "Agent 的回复会显示在这里"
                            } else if self.compact {
                                "发送后在这里查看 Agent 输出"
                            } else {
                                "从一个问题开始"
                            },
                            cx,
                        ))
                        .when(!self.compact && self.active.is_none(), |area| {
                            area.child(caption(
                                groups::path_label(self.selected_directory.as_deref()),
                                cx,
                            ))
                        }),
                )
            })
            .when(!empty, |area| {
                area.child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(
                            list(self.list.clone(), move |index, _, cx| {
                                view.update(cx, |this, cx| this.render_item(index, cx))
                            })
                            .size_full(),
                        )
                        .child(self.render_turn_navigation(cx)),
                )
            })
            .when(!self.approvals.is_empty(), |area| {
                area.child(
                    div()
                        .id("approval-panel")
                        .test_support()
                        .when(self.compact, |area| area.px_3())
                        .py_2()
                        .flex_shrink_0()
                        .max_h(px(APPROVAL_PANEL_HEIGHT))
                        .overflow_y_scroll()
                        .child(content_panel().child(self.approvals.render(
                            cx,
                            |this, request_id, outcome, _, cx| {
                                if let Some(session) = this.active {
                                    this.approvals.submitting(&request_id);
                                    this.command(Command::RespondPermission {
                                        session,
                                        request_id,
                                        outcome,
                                    });
                                    cx.notify();
                                }
                            },
                        ))),
                )
            })
            .children(self.render_queue_panel(cx))
            .when_some(self.authentication_panel.clone(), |area, panel| {
                area.child(
                    div()
                        .id("thread-authentication")
                        .test_support()
                        .when(self.compact, |area| area.px_3())
                        .py_2()
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .child(panel),
                )
            })
            .when(!self.elicitation_panels.is_empty(), |area| {
                area.child(
                    div()
                        .id("thread-elicitations")
                        .test_support()
                        .when(self.compact, |area| area.px_3())
                        .py_2()
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .children(self.elicitation_panels.values().cloned()),
                )
            })
            .into_any_element()
    }

    pub(super) fn stream_label(&self) -> String {
        if !self.approvals.is_empty() {
            return "等待工具审批…".into();
        }
        if self.approval_error.is_some() {
            return "工具审批不可用".into();
        }
        if !self.elicitation_panels.is_empty() {
            return "等待用户交互…".into();
        }
        if !self.connected {
            return if self.connection_error.is_some() {
                "连接已断开，正在重试…".into()
            } else {
                "正在连接 basrd…".into()
            };
        }
        if self.active.is_none() {
            return if self.creating {
                "正在创建 thread…".into()
            } else if self.waiting_for_agent_selection {
                "等待选择 Agent".into()
            } else if self.authentication_required.is_some() {
                "Agent 需要认证".into()
            } else if self.agents.is_empty() {
                "basrd 暂未配置可用的 Agent".into()
            } else {
                "Enter 发送 · Shift Enter 换行".into()
            };
        }
        if self.active.is_some_and(|id| self.configuring.contains(&id)) {
            return "正在应用选项…".into();
        }
        if self.active.is_some_and(|id| self.sending(id)) {
            return "正在发送…".into();
        }
        match &self.stream {
            StreamState::Loading => "正在加载对话…".into(),
            StreamState::Live if self.working() => self.work_status(),
            StreamState::Live => "已同步".into(),
            StreamState::Reconnecting(_) => "正在恢复连接…".into(),
            StreamState::Unavailable(_) => "无法读取 thread".into(),
            StreamState::Closed => "Thread 已结束".into(),
        }
    }

    pub(super) fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let fills_thread = self.composer_fills_thread();
        let panel = div();
        let line_height =
            (f32::from(theme::content_font_size(cx)) * theme::content_line_height(cx)).ceil();
        let expand_top = gpui_kit::component::Size::Medium.input_py()
            + px((line_height - layout::ICON_BUTTON_SIZE) / 2.);
        let sending = self.active.is_some_and(|id| self.sending(id));
        let cancelling = self.active.is_some_and(|id| self.cancelling.contains(&id));
        let can_send = self.can_send(cx);
        let submit_hint = if self.working() || cancelling {
            let hint = if cancelling {
                "正在停止…"
            } else {
                "停止当前任务"
            };
            self.composer_error(cx)
                .map(|error| format!("{error}\n\n{hint}"))
                .unwrap_or_else(|| hint.into())
        } else {
            self.send_hint(cx)
        };
        let submit = if self.working() || cancelling {
            Button::new("stop-turn")
                .ghost()
                .small()
                .icon(IconName::Square)
                .text_color(cx.theme().danger)
                .when(self.composer_error(cx).is_some(), |button| {
                    button.text_color(cx.theme().danger)
                })
                .disabled(
                    cancelling || self.connection_status() != status::ConnectionStatus::Connected,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(session) = this.active {
                        this.cancelling.insert(session);
                        this.outboxes.entry(session).or_default().pause();
                        this.command(Command::Cancel(session));
                        cx.notify();
                    }
                }))
        } else {
            Button::new("send-message")
                .primary()
                .when(!can_send, |button| button.ghost())
                .small()
                .icon(IconName::Send)
                .when(self.composer_error(cx).is_some(), |button| {
                    button.text_color(cx.theme().danger)
                })
                .disabled(!can_send)
                .on_click(cx.listener(|this, _, window, cx| this.send_prompt(window, cx)))
        };
        div()
            .flex_shrink_0()
            .w_full()
            .when(fills_thread, |area| {
                area.flex_1().min_h_0().flex().flex_col()
            })
            .when(self.compact, |area| {
                area.px(px(COMPOSER_SIDE_PADDING))
                    .pb(px(COMPOSER_BOTTOM_PADDING))
            })
            .when(!self.compact, |area| area.pb(px(COMPOSER_SIDE_PADDING)))
            .child(
                panel
                    .id("composer-panel")
                    .test_support()
                    .rounded_lg()
                    .flex()
                    .flex_col()
                    .when(fills_thread, |panel| panel.flex_1().min_h_0())
                    .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, cx| {
                        style.bg(cx.theme().primary.opacity(0.12))
                    })
                    .on_drop(
                        cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
                            this.drop_attachments(paths.paths().to_vec(), window, cx);
                        }),
                    )
                    .when(!self.compact, |input| {
                        input.border_1().border_color(cx.theme().border)
                    })
                    .child(self.render_commands(cx))
                    .child(
                        div()
                            .id("composer-editor")
                            .relative()
                            .key_context("BasrComposer")
                            .test_support()
                            .capture_action(cx.listener(
                                |this, _: &composer::ExpandMessageEditor, window, cx| {
                                    this.toggle_composer_expansion(window, cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .capture_action(cx.listener(
                                |this, _: &composer::SendComposer, window, cx| {
                                    this.submit_composer(window, cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                                this.capture_composer_enter(action, window, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &Paste, window, cx| {
                                this.paste_attachment(window, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &Backspace, window, cx| {
                                this.delete_mention(true, window, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &Delete, window, cx| {
                                this.delete_mention(false, window, cx)
                            }))
                            .capture_action(cx.listener(
                                |this, _: &DeleteToPreviousWordStart, window, cx| {
                                    this.delete_word_preserving_mentions(true, window, cx)
                                },
                            ))
                            .capture_action(cx.listener(
                                |this, _: &DeleteToNextWordEnd, window, cx| {
                                    this.delete_word_preserving_mentions(false, window, cx)
                                },
                            ))
                            .capture_action(
                                cx.listener(|this, _: &MoveLeft, _, cx| {
                                    this.cross_mention(true, cx)
                                }),
                            )
                            .capture_action(cx.listener(|this, _: &MoveRight, _, cx| {
                                this.cross_mention(false, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                                this.command_action("up", window, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                                this.command_action("down", window, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                                this.command_action("tab", window, cx)
                            }))
                            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                                this.command_action("escape", window, cx)
                            }))
                            .flex()
                            .items_start()
                            .pr(px(COMPOSER_CONTROL_INSET
                                + layout::ICON_BUTTON_SIZE
                                + COMPOSER_CONTROL_GAP))
                            .when(fills_thread, |editor| editor.flex_1().min_h_0())
                            .child(
                                div()
                                    .relative()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_x_hidden()
                                    .when(fills_thread, |editor| editor.h_full())
                                    .child(
                                        Editor::new(&self.composer)
                                            .w_auto()
                                            .flex_1()
                                            .min_w_0()
                                            // Trim the component's measured blank inset while
                                            // retaining its editing, selection and overlay behavior.
                                            .ml(-(self.composer_content_left.get()
                                                - px(COMPOSER_TEXT_INSET))
                                            .max(px(0.)))
                                            .font_family(cx.theme().font_family.clone())
                                            .text_size(theme::content_font_size(cx))
                                            .line_height(gpui_kit::relative(
                                                theme::content_line_height(cx),
                                            ))
                                            .appearance(false)
                                            .bordered(false)
                                            .readonly(sending || self.paste_pending)
                                            .aria_label("发送给 Agent 的消息")
                                            .when(fills_thread, |editor| editor.h_full())
                                            .when(!fills_thread, |editor| {
                                                editor.h(px(self.composer_height(cx)))
                                            }),
                                    )
                                    .child(self.render_attachment_chips(cx)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(expand_top.max(px(0.)))
                                    .right(px(COMPOSER_CONTROL_INSET))
                                    .child(self.composer_expansion_button(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("composer-toolbar")
                            .test_support()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_1()
                            .pl(px(COMPOSER_CONTROL_INSET))
                            .pr(px(COMPOSER_CONTROL_INSET))
                            .h(px(COMPOSER_TOOLBAR_HEIGHT))
                            .flex_shrink_0()
                            .children(self.render_usage(cx))
                            .child(
                                self.attachment_button(cx)
                                    .w(px(layout::ICON_BUTTON_SIZE))
                                    .h(px(layout::ICON_BUTTON_SIZE))
                                    .p_0(),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.render_config_options(cx)),
                            )
                            .children(self.render_queue(cx))
                            .when(
                                self.working() && !self.composer.read(cx).value().trim().is_empty(),
                                |toolbar| toolbar.child(self.immediate_send_button(cx)),
                            )
                            .when(
                                !self.compact && self.authentication_required.is_some(),
                                |toolbar| toolbar.child(self.authentication_button(cx)),
                            )
                            .when(
                                self.working() && !self.composer.read(cx).value().trim().is_empty(),
                                |toolbar| {
                                    toolbar.child(crate::tooltip::bounded(
                                        "queue-message-hint",
                                        Button::new("queue-message")
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Send)
                                            .disabled(!self.can_send(cx))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.send_prompt(window, cx)
                                            })),
                                        self.send_hint(cx),
                                    ))
                                },
                            )
                            .child(crate::tooltip::bounded(
                                "submit-hint",
                                submit
                                    .xsmall()
                                    .w(px(layout::ICON_BUTTON_SIZE))
                                    .h(px(layout::ICON_BUTTON_SIZE))
                                    .p_0()
                                    .flex_shrink_0(),
                                submit_hint,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_item(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(conversation) = self.conversation.as_ref() else {
            return div().into_any_element();
        };
        let Some(row) = conversation.rows.get(index) else {
            return div().into_any_element();
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
        div()
            .id(SharedString::from(id))
            .test_support()
            .when(self.compact, |row| {
                row.pl_3().pr(px(turn_navigation::RAIL_WIDTH))
            })
            .py_1()
            .w_full()
            .text_size(theme::content_font_size(cx))
            .child(
                content_panel()
                    .id(panel_id)
                    .test_support()
                    .child(content)
                    .children(
                        self.render_message_actions(
                            items
                                .last()
                                .expect("timeline rows contain at least one item"),
                            cx,
                        ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_entry(
        &self,
        item: &Item,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = item.id();
        match item {
            Item::Message { message, .. } if message.role == Role::Thought => {
                self.render_thought(&id, message, index, cx)
            }
            Item::Message { message, .. } => {
                let user = message.role == Role::User;
                let copy = message.plain_text();
                let group = SharedString::from(format!("message-{id}"));
                div()
                    .group(group.clone())
                    .relative()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .when(user, |message| {
                        message
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted.opacity(0.35))
                    })
                    .children(
                        message
                            .content
                            .iter()
                            .enumerate()
                            .map(|(block_index, block)| {
                                self.render_block(format!("{id}-{block_index}"), block, index, cx)
                            }),
                    )
                    .when(user, |row| {
                        row.child(
                            div()
                                .absolute()
                                .top_1()
                                .right_1()
                                .invisible()
                                .group_hover(group, |style| style.visible())
                                .child(
                                    Button::new(SharedString::from(format!("copy-{id}")))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Copy)
                                        .tooltip("复制消息")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                ),
                        )
                    })
                    .into_any_element()
            }
            Item::Tool(tool) => self.render_tool(&id, tool, index, cx),
            Item::Compaction(compaction) => self.render_compaction(&id, compaction, index, cx),
            Item::Notice { text, .. } => div()
                .px_4()
                .py_2()
                .child(caption(text.clone(), cx))
                .into_any_element(),
        }
    }

    pub(super) fn render_block(
        &self,
        id: String,
        block: &ContentBlock,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let markdown = block_source(block).map(|text| self.markdown(id.clone(), text, index, cx));
        let images = self.content_images(&id, index, cx);
        render_content(id, block, markdown, &images, cx)
    }

    fn disclosure(
        &self,
        id: &str,
        label: &str,
        expanded: bool,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Button {
        let key = id.to_owned();
        let line_height = (f32::from(theme::content_font_size(cx)) * 1.4).ceil();
        Button::new(SharedString::from(format!("expand-{id}")))
            .ghost()
            .small()
            .h(px((line_height + 8.).max(24.)))
            .icon(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .accessibility_label(label.to_owned())
            .child(
                div()
                    .id(SharedString::from(format!("disclosure-label-{id}")))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .text_size(theme::content_font_size(cx))
                    .line_height(px(line_height))
                    .truncate()
                    .child(label.to_owned()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.expanded.remove(&key) {
                    this.expanded.insert(key.clone());
                }
                this.list.remeasure_items(index..index + 1);
                cx.notify();
            }))
    }

    fn render_tool(
        &self,
        id: &str,
        tool: &ToolCall,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded.contains(id);
        let input_key = format!("input-open-{id}");
        let output_key = format!("output-closed-{id}");
        let input_open = self.expanded.contains(&input_key);
        let output_open = !self.expanded.contains(&output_key);
        let title = if tool.title.trim().is_empty() {
            tool.name.as_deref().unwrap_or("工具调用")
        } else {
            &tool.title
        };
        let (label, status_icon, color) = match tool.status {
            ToolCallStatus::Pending => ("等待中", IconName::Clock, cx.theme().muted_foreground),
            ToolCallStatus::InProgress => ("执行中", IconName::Loader, cx.theme().primary),
            ToolCallStatus::Completed => ("完成", IconName::Check, cx.theme().success),
            ToolCallStatus::Failed => ("失败", IconName::CircleX, cx.theme().danger),
            _ => ("未知", IconName::CircleAlert, cx.theme().muted_foreground),
        };
        div()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .on_prepaint(self.track_content_header(id))
                    .child(
                        crate::tooltip::bounded(
                            SharedString::from(format!("tool-hint-{id}")),
                            div()
                                .id(SharedString::from(format!("tool-command-{id}")))
                                .test_support()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(
                                    self.disclosure(
                                        id,
                                        &title.split_whitespace().collect::<Vec<_>>().join(" "),
                                        expanded,
                                        index,
                                        cx,
                                    )
                                    .w_full(),
                                ),
                            title.to_owned(),
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(
                        Button::new(SharedString::from(format!("status-{id}")))
                            .ghost()
                            .xsmall()
                            .w(px(20.))
                            .h(px(20.))
                            .p_0()
                            .flex_none()
                            .tooltip(label)
                            .child(status::activity_icon(
                                SharedString::from(format!("tool-status-animation-{id}")),
                                status_icon,
                                color,
                                tool.status == ToolCallStatus::InProgress,
                            )),
                    ),
            )
            .when(expanded, |card| {
                card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .px_3()
                        .pb_3()
                        .child(
                            div()
                                .id(SharedString::from(format!("tool-output-section-{id}")))
                                .test_support()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    self.disclosure(&output_key, "输出", output_open, index, cx)
                                        .w_full()
                                        .font_weight(FontWeight::SEMIBOLD),
                                )
                                .when(output_open, |section| {
                                    section.child(self.render_tool_output(id, tool, index, cx))
                                }),
                        )
                        .when(self.tool_has_input(tool), |body| {
                            body.child(
                                div()
                                    .id(SharedString::from(format!("tool-input-section-{id}")))
                                    .test_support()
                                    .rounded_md()
                                    .bg(cx.theme().muted.opacity(0.25))
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        self.disclosure(&input_key, "输入", input_open, index, cx)
                                            .w_full()
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .when(input_open, |section| {
                                        section.child(self.render_tool_input(id, tool, index, cx))
                                    }),
                            )
                        })
                        .child(self.render_content_actions(id, id, index, cx)),
                )
            })
            .into_any_element()
    }

    fn tool_has_input(&self, tool: &ToolCall) -> bool {
        tool.raw_input.is_some()
            || tool.name.is_some()
            || tool.title.contains(['\r', '\n'])
            || tool.title.chars().count() > 96
            || tool.content.iter().any(|content| {
                let acp::ToolCallContent::Terminal(reference) = content else {
                    return false;
                };
                self.conversation
                    .as_ref()
                    .and_then(|c| c.terminals.get(reference.terminal_id.0.as_ref()))
                    .is_some_and(|terminal| terminal.label.is_some() || terminal.cwd.is_some())
            })
    }

    fn render_tool_input(
        &self,
        id: &str,
        tool: &ToolCall,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title_is_terminal_command = tool.content.iter().any(|content| {
            let acp::ToolCallContent::Terminal(reference) = content else {
                return false;
            };
            self.conversation
                .as_ref()
                .and_then(|c| c.terminals.get(reference.terminal_id.0.as_ref()))
                .and_then(|terminal| terminal.label.as_ref())
                .is_some_and(|label| label.trim() == tool.title.trim())
        });
        div()
            .id(SharedString::from(format!("tool-input-{id}")))
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .pb_3()
            .when_some(tool.name.as_ref(), |body, name| {
                body.child(
                    div()
                        .id(SharedString::from(format!("tool-name-{id}")))
                        .test_support()
                        .child(caption(format!("工具：{name}"), cx)),
                )
            })
            .when(
                !tool
                    .raw_input
                    .as_ref()
                    .is_some_and(|input| input_contains_command(input, &tool.title))
                    && !title_is_terminal_command
                    && (tool.title.contains(['\r', '\n']) || tool.title.chars().count() > 96),
                |body| {
                    body.child(self.markdown(
                        format!("{id}-command"),
                        code_block("sh", &tool.title),
                        index,
                        cx,
                    ))
                },
            )
            .when_some(tool.raw_input.as_ref(), |body, input| {
                if let Some(command) = input.get("command").and_then(serde_json::Value::as_str) {
                    let mut parameters = input
                        .as_object()
                        .expect("command belongs to an object")
                        .clone();
                    parameters.remove("command");
                    body.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(caption("命令", cx))
                            .child(self.markdown(
                                format!("{id}-input-command"),
                                code_block("sh", command),
                                index,
                                cx,
                            ))
                            .when(!parameters.is_empty(), |input| {
                                input.child(caption("参数", cx)).child(self.markdown(
                                    format!("{id}-input"),
                                    json_block(&serde_json::Value::Object(parameters)),
                                    index,
                                    cx,
                                ))
                            }),
                    )
                } else {
                    body.child(self.markdown(format!("{id}-input"), json_block(input), index, cx))
                }
            })
            .children(tool.content.iter().filter_map(|content| {
                let acp::ToolCallContent::Terminal(reference) = content else {
                    return None;
                };
                let terminal = self
                    .conversation
                    .as_ref()?
                    .terminals
                    .get(reference.terminal_id.0.as_ref())?;
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .when_some(
                            terminal.label.as_ref().filter(|label| {
                                !tool
                                    .raw_input
                                    .as_ref()
                                    .is_some_and(|input| input_contains_command(input, label))
                            }),
                            |body, label| {
                                body.child(self.markdown(
                                    format!("{id}-terminal-command-{}", reference.terminal_id),
                                    code_block("sh", label),
                                    index,
                                    cx,
                                ))
                            },
                        )
                        .when_some(terminal.cwd.as_ref(), |body, cwd| {
                            body.child(
                                div()
                                    .id(SharedString::from(format!(
                                        "terminal-cwd-{}",
                                        reference.terminal_id
                                    )))
                                    .test_support()
                                    .child(caption(format!("工作目录：{}", cwd.display()), cx)),
                            )
                        }),
                )
            }))
            .into_any_element()
    }

    fn render_tool_output(
        &self,
        id: &str,
        tool: &ToolCall,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(SharedString::from(format!("tool-output-{id}")))
            .test_support()
            .flex()
            .flex_col()
            .gap_3()
            .pl_3()
            .border_l_2()
            .border_color(cx.theme().primary.opacity(0.55))
            .when_some(
                tool.raw_output
                    .as_ref()
                    .filter(|_| !self.tool_has_rendered_output(tool)),
                |body, output| {
                    body.child(self.markdown(format!("{id}-output"), json_block(output), index, cx))
                },
            )
            .children(tool.locations.iter().map(|location| {
                caption(
                    format!(
                        "{}{}",
                        location.path.display(),
                        location
                            .line
                            .map(|line| format!(":{line}"))
                            .unwrap_or_default()
                    ),
                    cx,
                )
            }))
            .children(
                tool.content
                    .iter()
                    .enumerate()
                    .map(|(part, content)| match content {
                        acp::ToolCallContent::Content(content) => self.render_block(
                            format!("{id}-content-{part}"),
                            &content.content,
                            index,
                            cx,
                        ),
                        acp::ToolCallContent::Diff(diff) => self.markdown(
                            format!("{id}-diff-{part}"),
                            tool_content::legacy_diff(diff),
                            index,
                            cx,
                        ),
                        acp::ToolCallContent::Terminal(terminal) => {
                            self.render_terminal(id, terminal, tool, index, cx)
                        }
                        _ => caption("此工具内容暂不支持预览", cx).into_any_element(),
                    }),
            )
            .when(
                tool.content.is_empty() && tool.raw_output.is_none(),
                |body| {
                    body.child(caption(
                        if matches!(
                            tool.status,
                            ToolCallStatus::Completed | ToolCallStatus::Failed
                        ) {
                            "此调用未提供输出"
                        } else {
                            "等待工具输出…"
                        },
                        cx,
                    ))
                },
            )
            .into_any_element()
    }
    fn tool_has_rendered_output(&self, tool: &ToolCall) -> bool {
        tool.content.iter().any(|content| match content {
            acp::ToolCallContent::Content(_) | acp::ToolCallContent::Diff(_) => true,
            acp::ToolCallContent::Terminal(terminal) => self
                .conversation
                .as_ref()
                .and_then(|c| c.terminals.get(terminal.terminal_id.0.as_ref()))
                .is_some_and(|terminal| !terminal.output.is_empty()),
            _ => false,
        })
    }

    fn render_terminal(
        &self,
        id: &str,
        reference: &acp::Terminal,
        tool: &ToolCall,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let terminal = self
            .conversation
            .as_ref()
            .and_then(|c| c.terminals.get(reference.terminal_id.0.as_ref()));
        if let Some(terminal) = terminal {
            div()
                .id(SharedString::from(format!(
                    "terminal-{}",
                    reference.terminal_id
                )))
                .test_support()
                .flex()
                .flex_col()
                .gap_2()
                .when(!terminal.output.is_empty(), |body| {
                    body.child(self.markdown(
                        format!("{id}-terminal-{}", reference.terminal_id),
                        code_block("text", &terminal.output),
                        index,
                        cx,
                    ))
                })
                .when_some(terminal.exit.as_ref(), |body, exit| {
                    let mut status = Vec::new();
                    if let Some(code) = exit.exit_code {
                        status.push(format!("退出码 {code}"));
                    }
                    if let Some(signal) = &exit.signal {
                        status.push(format!("信号 {signal}"));
                    }
                    body.child(
                        div()
                            .id(SharedString::from(format!(
                                "terminal-exit-{}",
                                reference.terminal_id
                            )))
                            .test_support()
                            .child(caption(
                                if status.is_empty() {
                                    "已退出".to_owned()
                                } else {
                                    status.join(" · ")
                                },
                                cx,
                            )),
                    )
                })
                .when(
                    terminal.output.is_empty()
                        && terminal.exit.is_none()
                        && tool.raw_output.is_none(),
                    |body| {
                        body.child(caption(
                            if matches!(
                                tool.status,
                                ToolCallStatus::Completed | ToolCallStatus::Failed
                            ) {
                                "此调用未提供可读取的终端输出"
                            } else {
                                "等待终端输出…"
                            },
                            cx,
                        ))
                    },
                )
                .into_any_element()
        } else if tool.raw_output.is_some() {
            div().into_any_element()
        } else {
            caption(
                if matches!(
                    tool.status,
                    ToolCallStatus::Completed | ToolCallStatus::Failed
                ) {
                    "此调用未提供可读取的终端输出"
                } else {
                    "等待终端输出…"
                },
                cx,
            )
            .into_any_element()
        }
    }
}

fn input_contains_command(input: &serde_json::Value, command: &str) -> bool {
    input
        .as_str()
        .or_else(|| input.get("command").and_then(serde_json::Value::as_str))
        .is_some_and(|input| input.trim() == command.trim())
}

fn json_block(value: &serde_json::Value) -> String {
    if let Some(text) = value.as_str() {
        return code_block("text", text);
    }
    code_block(
        "json",
        &serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    )
}

fn code_block(language: &str, text: &str) -> String {
    let fence = "~".repeat(
        text.split(|ch| ch != '~')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    format!("{fence}{language}\n{text}\n{fence}")
}

pub(super) fn block_source(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(text) => Some(text.text.clone()),
        ContentBlock::Resource(resource) => match &resource.resource {
            acp::EmbeddedResourceResource::TextResourceContents(text) => {
                Some(crate::attachment::file_code(&text.text, &text.uri))
            }
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn render_content(
    id: String,
    block: &ContentBlock,
    markdown: Option<AnyElement>,
    images: &Entity<images::Cache>,
    cx: &App,
) -> AnyElement {
    match block {
        ContentBlock::Text(_) => div().children(markdown).into_any_element(),
        ContentBlock::ResourceLink(resource) => {
            let uri = resource.uri.clone();
            let group = SharedString::from(format!("resource-copy-{id}"));
            div()
                .group(group.clone())
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .p_3()
                .flex()
                .gap_3()
                .items_center()
                .child(Icon::new(IconName::File).small())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(resource.name.clone())
                        .child(caption(uri.clone(), cx).truncate()),
                )
                .child(
                    message_actions::action_button(SharedString::from(id), IconName::Copy, cx)
                        .tooltip("复制资源地址")
                        .invisible()
                        .group_hover(group, |style| style.visible())
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(uri.clone()))
                        }),
                )
                .into_any_element()
        }
        ContentBlock::Image(image) => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(caption(format!("图片 · {}", image.mime_type), cx))
            .child(images::acp_image(&id, image, images, false, cx))
            .into_any_element(),
        ContentBlock::Resource(resource) => match &resource.resource {
            acp::EmbeddedResourceResource::TextResourceContents(text) => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(caption(text.uri.clone(), cx))
                .children(markdown)
                .into_any_element(),
            acp::EmbeddedResourceResource::BlobResourceContents(blob) => caption(
                format!(
                    "文件 · {} · {}",
                    blob.uri,
                    blob.mime_type
                        .as_deref()
                        .unwrap_or("application/octet-stream")
                ),
                cx,
            )
            .into_any_element(),
            _ => caption("嵌入资源", cx).into_any_element(),
        },
        ContentBlock::Audio(audio) => {
            caption(format!("音频 · {}", audio.mime_type), cx).into_any_element()
        }
        _ => caption("内容暂不支持预览", cx).into_any_element(),
    }
}
