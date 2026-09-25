use base64::Engine;
use basr::acp;
use basr::model::{ContentBlock, Role, ToolCall, ToolCallStatus};
use gpui_kit::{Image, ImageFormat, img};
use std::sync::Arc;

use super::*;

pub(super) const CONTENT_WIDTH: f32 = 840.;
pub(super) const COMPOSER_TOOLBAR_HEIGHT: f32 = 24.;
pub(super) const COMPOSER_BOTTOM_PADDING: f32 = 2.;

pub(super) fn content_panel() -> Div {
    div().w_full().max_w(px(CONTENT_WIDTH)).mx_auto()
}

impl Workspace {
    pub(super) fn composer_height(&self, cx: &App) -> f32 {
        let input = self.composer.read(cx);
        // Height follows explicit line breaks; upstream does not expose wrapped row counts.
        let rows = input
            .text()
            .lines_len()
            .clamp(1, if self.compact { 4 } else { 10 });
        rows as f32
            * (f32::from(theme::content_font_size(cx)) * theme::content_line_height(cx)).ceil()
            + f32::from(gpui_kit::component::Size::Medium.input_py()) * 2.
    }

    fn work_status(&self) -> String {
        if let Some(conversation) = &self.conversation {
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
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(self.render_timeline(cx))
            .child(
                div().px_3().child(
                    content_panel()
                        .id("conversation-status")
                        .test_support()
                        .flex()
                        .items_center()
                        .gap_2()
                        .py_1()
                        .child(self.connection_indicator("conversation-connection", cx))
                        .children(
                            self.activity_label()
                                .map(|label| caption(label, cx).flex_1().min_w_0().truncate()),
                        )
                        .child(div().flex_1())
                        .when(self.active.is_some(), |row| {
                            row.child(
                                Button::new("scroll-to-bottom")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowDown)
                                    .tooltip("滚动到最新输出")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.list.set_follow_mode(FollowMode::Tail);
                                        cx.notify();
                                    })),
                            )
                        }),
                ),
            )
            .child(self.render_composer(cx))
            .into_any_element()
    }

    pub(super) fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let empty = self
            .conversation
            .as_ref()
            .is_none_or(|conversation| conversation.items.is_empty());
        div()
            .flex_1()
            .min_h_0()
            .size_full()
            .when(empty, |area| {
                area.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .items_center()
                        .gap_2()
                        .p_5()
                        .child(caption(
                            if self.creating {
                                "正在创建 thread…"
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
                    list(self.list.clone(), move |index, window, cx| {
                        view.update(cx, |this, cx| this.render_item(index, window, cx))
                    })
                    .size_full(),
                )
            })
            .into_any_element()
    }

    pub(super) fn stream_label(&self) -> String {
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
            } else if self.authentication_required.is_some() {
                "Agent 已连接，需要认证".into()
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
        let sending = self.active.is_some_and(|id| self.sending(id));
        let cancelling = self.active.is_some_and(|id| self.cancelling.contains(&id));
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
                .when(!self.can_send(cx), |button| button.ghost())
                .small()
                .icon(IconName::Send)
                .when(self.composer_error(cx).is_some(), |button| {
                    button.text_color(cx.theme().danger)
                })
                .disabled(!self.can_send(cx))
                .on_click(cx.listener(|this, _, window, cx| this.send_prompt(window, cx)))
        };
        div()
            .flex_shrink_0()
            .w_full()
            .when(self.compact, |area| {
                area.px_3().pb(px(COMPOSER_BOTTOM_PADDING))
            })
            .when(!self.compact, |area| area.px_3().pb_3())
            .child(
                content_panel()
                    .id("composer-panel")
                    .test_support()
                    .rounded_lg()
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
                            .test_support()
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
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        Editor::new(&self.composer)
                                            .font_family(cx.theme().font_family.clone())
                                            .text_size(theme::content_font_size(cx))
                                            .line_height(gpui_kit::relative(
                                                theme::content_line_height(cx),
                                            ))
                                            .appearance(false)
                                            .bordered(false)
                                            .readonly(sending)
                                            .aria_label("发送给 Agent 的消息")
                                            .h(px(self.composer_height(cx))),
                                    )
                                    .child(self.render_attachment_chips(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("composer-toolbar")
                            .test_support()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px(gpui_kit::component::Size::Medium.input_px())
                            .h(px(COMPOSER_TOOLBAR_HEIGHT))
                            .flex_shrink_0()
                            .child(self.attachment_button(cx))
                            .child(self.agent_button(cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.render_config_options(cx)),
                            )
                            .children(self.render_queue(cx))
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
                                submit.xsmall().flex_shrink_0(),
                                submit_hint,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_item(&self, index: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.items.get(index))
        else {
            return div().into_any_element();
        };
        let id = item.id();
        let content = match item {
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
                        message.border_1().border_color(cx.theme().border)
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
                    .child(
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
                    .into_any_element()
            }
            Item::Tool(tool) => self.render_tool(&id, tool, index, cx),
            Item::Notice { text, .. } => div()
                .px_4()
                .py_2()
                .child(caption(text.clone(), cx))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(id))
            .test_support()
            .px_3()
            .py_1()
            .w_full()
            .text_size(theme::content_font_size(cx))
            .child(content_panel().child(content))
            .into_any_element()
    }

    pub(super) fn render_block(
        &self,
        id: String,
        block: &ContentBlock,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let markdown = block_source(block).map(|text| self.markdown(id.clone(), text, index, cx));
        render_content(id, block, markdown, cx)
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
        Button::new(SharedString::from(format!("expand-{id}")))
            .ghost()
            .small()
            .icon(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .accessibility_label(label.to_owned())
            .child(div().flex_1().min_w_0().truncate().child(label.to_owned()))
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
                                        &tool
                                            .title
                                            .split_whitespace()
                                            .collect::<Vec<_>>()
                                            .join(" "),
                                        expanded,
                                        index,
                                        cx,
                                    )
                                    .w_full(),
                                ),
                            tool.title.clone(),
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(
                        Button::new(SharedString::from(format!("status-{id}")))
                            .ghost()
                            .xsmall()
                            .flex_none()
                            .icon(status_icon)
                            .text_color(color)
                            .tooltip(label),
                    ),
            )
            .when(expanded, |card| {
                card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .p_4()
                        .when(tool.title.contains(['\r', '\n']), |body| {
                            body.child(self.markdown(
                                format!("{id}-command"),
                                code_block("sh", &tool.title),
                                index,
                                cx,
                            ))
                        })
                        .when_some(tool.raw_input.as_ref(), |body, input| {
                            body.child(div().child(caption("参数", cx)).child(self.markdown(
                                format!("{id}-input"),
                                json_block(input),
                                index,
                                cx,
                            )))
                        })
                        .when_some(
                            tool.raw_output
                                .as_ref()
                                .filter(|_| !self.tool_has_rendered_output(tool)),
                            |body, output| {
                                body.child(div().child(caption("结果", cx)).child(self.markdown(
                                    format!("{id}-output"),
                                    json_block(output),
                                    index,
                                    cx,
                                )))
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
                        .children(tool.content.iter().enumerate().map(|(part, content)| {
                            match content {
                                acp::ToolCallContent::Content(content) => self.render_block(
                                    format!("{id}-content-{part}"),
                                    &content.content,
                                    index,
                                    cx,
                                ),
                                acp::ToolCallContent::Diff(diff) => self
                                    .markdown(
                                        format!("{id}-diff-{part}"),
                                        format!(
                                            "{}\n\n\x60\x60\x60diff\n{}\n\x60\x60\x60",
                                            diff.path.display(),
                                            diff.new_text
                                        ),
                                        index,
                                        cx,
                                    )
                                    .into_any_element(),
                                acp::ToolCallContent::Terminal(terminal) => {
                                    self.render_terminal(id, terminal, tool, index, cx)
                                }
                                _ => caption("此工具内容暂不支持预览", cx).into_any_element(),
                            }
                        })),
                )
            })
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
        if let Some(terminal) = terminal.filter(|terminal| !terminal.output.is_empty()) {
            div()
                .id(SharedString::from(format!(
                    "terminal-{}",
                    reference.terminal_id
                )))
                .test_support()
                .child(self.markdown(
                    format!("{id}-terminal-{}", reference.terminal_id),
                    code_block("text", &terminal.output),
                    index,
                    cx,
                ))
                .when_some(terminal.exit.as_ref(), |body, exit| {
                    body.child(caption(
                        exit.exit_code
                            .map(|code| format!("退出码 {code}"))
                            .or_else(|| exit.signal.as_ref().map(|signal| format!("信号 {signal}")))
                            .unwrap_or_default(),
                        cx,
                    ))
                })
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

pub(super) fn decode_image(image: &acp::ImageContent) -> Option<Arc<Image>> {
    let format = match image.mime_type.as_str() {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::Webp,
        "image/gif" => ImageFormat::Gif,
        "image/svg+xml" => ImageFormat::Svg,
        "image/bmp" => ImageFormat::Bmp,
        "image/tiff" => ImageFormat::Tiff,
        "image/x-icon" | "image/vnd.microsoft.icon" => ImageFormat::Ico,
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .ok()?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
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
    markdown: Option<TextView>,
    cx: &App,
) -> AnyElement {
    match block {
        ContentBlock::Text(_) => div().children(markdown).into_any_element(),
        ContentBlock::ResourceLink(resource) => {
            let uri = resource.uri.clone();
            div()
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
                    Button::new(SharedString::from(id))
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .tooltip("复制资源地址")
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
            .children(decode_image(image).map(|image| img(image).max_w_full()))
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
