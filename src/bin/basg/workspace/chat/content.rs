use bas::acp;
use bas::model::{ContentBlock, Role, ToolCall, ToolCallStatus};

use super::super::*;

pub(in crate::workspace) fn message_block_id(
    message: &bas::model::Message,
    index: usize,
) -> String {
    format!("message-{}-{index}", message.id)
}

fn tool_icon(kind: bas::model::ToolKind) -> IconName {
    use bas::model::ToolKind;
    match kind {
        ToolKind::Read => IconName::FileText,
        ToolKind::Edit => IconName::FilePen,
        ToolKind::Delete => IconName::Trash,
        ToolKind::Move => IconName::Folder,
        ToolKind::Search => IconName::Search,
        ToolKind::Execute => IconName::Terminal,
        ToolKind::Think => IconName::Lightbulb,
        ToolKind::Fetch => IconName::Globe,
        _ => IconName::Wrench,
    }
}

impl Workspace {
    pub(in crate::workspace) fn render_entry(
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
                gds_uim::el! {
                    #div {
                        id: $SharedString::from(format!("message-surface-{id}"));
                        test_support; w_full; min_w_0;
                        group: $group.clone();
                        relative; flex; flex_col; gap_2; px_3; py_2; rounded_md;
                        @user => {
                            border_1;
                            border_color: $cx.theme().border;
                            bg: $cx.theme().muted.opacity(0.35);
                        };
                    } [
                        ..$message
                            .content
                            .iter()
                            .enumerate()
                            .map(|(block_index, block)| {
                                self.render_block(message_block_id(message, block_index), block, index, cx)
                            }),
                        ..$user.then(|| gds_uim::el! {
                            #div {
                                absolute; top_1; right_1; invisible;
                                group_hover: $group, $gds_uim::m! { visible; };
                            } [
                                $message_actions::action_button(SharedString::from(format!("copy-{id}")), IconName::Copy, cx) {
                                    accessibility_label: $crate::i18n::tr!("action-copy-message");
                                    tooltip: $crate::i18n::tr!("action-copy-message");
                                    on_click: $move |_, _, cx| copy_chat_text(&copy, cx);
                                },
                            ]
                        }),
                    ]
                }
                .into_any_element()
            }
            Item::Tool(tool) => self.render_tool(&id, tool, index, cx),
            Item::Compaction(compaction) => self.render_compaction(&id, compaction, index, cx),
            Item::Notice { text, .. } => gds_uim::el! {
                #div { px_3; py_2; flex; items_start; gap_2; } [
                    Icon($IconName::Info) {
                        size: $px(crate::ui::ICON);
                        text_color: $cx.theme().muted_foreground;
                        flex_shrink_0;
                    },
                    $caption(text.clone(), cx) { flex_1; min_w_0; },
                ]
            }
            .into_any_element(),
        }
    }

    pub(in crate::workspace) fn render_block(
        &self,
        id: String,
        block: &ContentBlock,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let markdown = block_source(block).map(|text| self.markdown(id.clone(), text, index, cx));
        let images = self.content_images(&id, index, cx);
        render_content(
            id,
            block,
            markdown,
            &images,
            &self.audio,
            self.backend.server.ip().is_loopback(),
            cx,
        )
    }

    pub(in crate::workspace) fn disclosure(
        &self,
        id: &str,
        label: &str,
        expanded: bool,
        index: usize,
        icon: Option<IconName>,
        cx: &mut Context<Self>,
    ) -> Button {
        let key = id.to_owned();
        let line_height = (f32::from(theme::content_font_size(cx)) * 1.4).ceil();
        gds_uim::el! {
            Button($SharedString::from(format!("expand-{id}"))) {
                ghost; small;
                h: $px((line_height + 8.).max(24.));
                icon: $(if expanded { IconName::ChevronDown } else { IconName::ChevronRight });
                on_prepaint: $self.track_content_header(id);
                accessibility_label: $label.to_owned();
            } [
                ..$icon.map(|icon| gds_uim::el! {
                    Icon($icon) {
                        size: $px(crate::ui::ICON);
                        text_color: $cx.theme().muted_foreground;
                        flex_shrink_0;
                    }
                }),
                #div {
                    id: $SharedString::from(format!("disclosure-label-{id}"));
                    test_support; flex_1; min_w_0;
                    text_size: $theme::content_font_size(cx);
                    line_height: $px(line_height);
                    truncate;
                } [$label.to_owned()],
            ].gds_uim::m! {
                on_click: $cx.listener(move |this, _, _, cx| {
                    this.toggle_content(&key, index, cx);
                });
            }
        }
    }

    fn render_tool(
        &self,
        id: &str,
        tool: &ToolCall,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.selected.timeline.expanded.contains(id);
        let input_key = format!("input-open-{id}");
        let output_key = format!("output-closed-{id}");
        let input_open = self.selected.timeline.expanded.contains(&input_key);
        let output_open = !self.selected.timeline.expanded.contains(&output_key);
        let title = if tool.title.trim().is_empty() {
            tool.name
                .as_deref()
                .unwrap_or(crate::i18n::tr!("tool-call"))
        } else {
            &tool.title
        };
        let (label, status_icon, color) = match tool.status {
            ToolCallStatus::Pending => (
                crate::i18n::tr!("status-pending"),
                IconName::Clock,
                cx.theme().muted_foreground,
            ),
            ToolCallStatus::InProgress => (
                crate::i18n::tr!("status-running"),
                IconName::Loader,
                cx.theme().primary,
            ),
            ToolCallStatus::Completed => (
                crate::i18n::tr!("status-completed"),
                IconName::Check,
                cx.theme().success,
            ),
            ToolCallStatus::Failed => (
                crate::i18n::tr!("status-failed"),
                IconName::CircleX,
                cx.theme().danger,
            ),
            _ => (
                crate::i18n::tr!("status-unknown"),
                IconName::CircleAlert,
                cx.theme().muted_foreground,
            ),
        };
        let command = gds_uim::el! {
            #div {
                id: $SharedString::from(format!("tool-command-{id}"));
                test_support; flex_1; min_w_0; overflow_hidden;
            } [
                $self.disclosure(
                    id,
                    &title.split_whitespace().collect::<Vec<_>>().join(" "),
                    expanded,
                    index,
                    Some(tool_icon(tool.kind)),
                    cx,
                ) { w_full; },
            ]
        };
        gds_uim::el! {
            #div {
                rounded_md; w_full; min_w_0; border_1;
                border_color: $cx.theme().border;
                overflow_hidden;
            } [
                #div {
                    px_2; py_1;
                    on_prepaint: $self.track_content_header(id);
                } [
                    #crate::ui::ContentRow::new(
                        $format!("tool-row-{id}"),
                        $crate::ui::tooltip::bounded(
                            SharedString::from(format!("tool-hint-{id}")),
                            command,
                            title.to_owned(),
                        )
                    ) [
                        Button($SharedString::from(format!("status-{id}"))) {
                            ghost; xsmall; cursor_default;
                            w: $px(crate::ui::COMPACT_CONTROL);
                            h: $px(crate::ui::COMPACT_CONTROL);
                            p_0; flex_none;
                            tooltip: $label;
                        } [
                            $status::activity_icon(
                                SharedString::from(format!("tool-status-animation-{id}")),
                                status_icon,
                                color,
                                tool.status == ToolCallStatus::InProgress,
                            ),
                        ],
                    ],
                ],
                ..$expanded.then(|| {
                    gds_uim::el! {
                        #div { flex; flex_col; gap_3; px_3; pb_3; } [
                            ..$self.tool_has_input(tool).then(|| gds_uim::el! {
                                #div {
                                    id: $SharedString::from(format!("tool-input-section-{id}"));
                                    test_support; flex; flex_col; gap_2;
                                } [
                                        $self.disclosure(
                                            &input_key,
                                            crate::i18n::tr!("tool-input"),
                                            input_open,
                                            index,
                                            None,
                                            cx,
                                        ) { w_full; text_color: $cx.theme().muted_foreground; },
                                        ..$input_open.then(|| self.render_tool_input(id, tool, index, cx)),
                                ]
                            }),
                            #div {
                                id: $SharedString::from(format!("tool-output-section-{id}"));
                                test_support; flex; flex_col; gap_2;
                            } [
                                    $self.disclosure(
                                        &output_key,
                                        crate::i18n::tr!("tool-output"),
                                        output_open,
                                        index,
                                        None,
                                        cx,
                                    ) { w_full; text_color: $cx.theme().muted_foreground; },
                                    ..$output_open.then(|| self.render_tool_output(id, tool, index, cx)),
                            ],
                            $self.render_content_actions(id, id, index, cx),
                        ]
                    }
                }),
            ]
        }.into_any_element()
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
                self.selected
                    .timeline
                    .conversation
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
            self.selected
                .timeline
                .conversation
                .as_ref()
                .and_then(|c| c.terminals.get(reference.terminal_id.0.as_ref()))
                .and_then(|terminal| terminal.label.as_ref())
                .is_some_and(|label| label.trim() == tool.title.trim())
        });
        gds_uim::el! {
            #div {
                id: $SharedString::from(format!("tool-input-{id}"));
                test_support; flex; flex_col; gap_2; px_3; pb_3;
            } [
                ..$tool.name.as_ref().map(|name| gds_uim::el! {
                    #div {
                        id: $SharedString::from(format!("tool-name-{id}"));
                        test_support;
                    } [
                        $caption(
                            crate::i18n::tr!("tool-name", name = (name).to_string()),
                            cx,
                        ),
                    ]
                }),
                ..$((
                !tool
                    .raw_input
                    .as_ref()
                    .is_some_and(|input| input_contains_command(input, &tool.title))
                    && !title_is_terminal_command
                    && (tool.title.contains(['\r', '\n']) || tool.title.chars().count() > 96))
                    .then(|| self.markdown(
                        format!("{id}-command"),
                        code_block("sh", &tool.title),
                        index,
                        cx,
                    ))),
                ..$tool.raw_input.as_ref().map(|input| self.render_tool_parameters(id, input, index, cx)),
                ..$tool.content.iter().filter_map(|content| self.render_terminal_input(id, tool, content, index, cx)),
            ]
        }
        .into_any_element()
    }

    fn render_tool_parameters(
        &self,
        id: &str,
        input: &serde_json::Value,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(command) = input.get("command").and_then(serde_json::Value::as_str) else {
            return self.markdown(format!("{id}-input"), json_block(input), index, cx);
        };
        let mut parameters = input
            .as_object()
            .expect("command belongs to an object")
            .clone();
        parameters.remove("command");
        gds_uim::el! {
            #div { flex; flex_col; gap_2; } [
                $caption(crate::i18n::tr!("tool-command"), cx),
                $self.markdown(format!("{id}-input-command"), code_block("sh", command), index, cx),
            ].gds_uim::m! {
                @$(!parameters.is_empty()) => {
                    child: $caption(crate::i18n::tr!("tool-arguments"), cx);
                    child: $self.markdown(
                        format!("{id}-input"),
                        json_block(&serde_json::Value::Object(parameters)),
                        index,
                        cx,
                    );
                };
            }
        }
        .into_any_element()
    }

    fn render_terminal_input(
        &self,
        id: &str,
        tool: &ToolCall,
        content: &acp::ToolCallContent,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let acp::ToolCallContent::Terminal(reference) = content else {
            return None;
        };
        let terminal = self
            .selected
            .timeline
            .conversation
            .as_ref()?
            .terminals
            .get(reference.terminal_id.0.as_ref())?;
        Some(gds_uim::el! {
            #div { flex; flex_col; gap_2; } [
                ..$terminal.label.as_ref().filter(|label| {
                    !tool.raw_input.as_ref().is_some_and(|input| input_contains_command(input, label))
                }).map(|label| {
                    self.markdown(
                        format!("{id}-terminal-command-{}", reference.terminal_id),
                        code_block("sh", label),
                        index,
                        cx,
                    )
                }),
                ..$terminal.cwd.as_ref().map(|cwd| gds_uim::el! {
                    #div {
                        id: $SharedString::from(format!("terminal-cwd-{}", reference.terminal_id));
                        test_support;
                    } [
                        $caption(
                            crate::i18n::tr!("tool-directory", path = (cwd.display()).to_string()),
                            cx,
                        ),
                    ]
                }),
            ]
        }.into_any_element())
    }

    fn render_tool_output(
        &self,
        id: &str,
        tool: &ToolCall,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        gds_uim::el! {
            #div {
                id: $SharedString::from(format!("tool-output-{id}"));
                test_support; flex; flex_col; gap_3; pl_3; border_l_2;
                border_color: $cx.theme().primary.opacity(0.55);
            } [
                ..$tool.raw_output
                    .as_ref()
                    .filter(|_| !self.tool_has_rendered_output(tool))
                    .map(|output| self.markdown(format!("{id}-output"), json_block(output), index, cx)),
                ..$tool.locations.iter().map(|location| {
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
                }),
                ..$tool.content
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
                        _ => caption(crate::i18n::tr!("tool-preview-unavailable"), cx)
                            .into_any_element(),
                    }),
                ..$((tool.content.is_empty() && tool.raw_output.is_none()).then(|| {
                    caption(
                        if matches!(
                            tool.status,
                            ToolCallStatus::Completed | ToolCallStatus::Failed
                        ) {
                            crate::i18n::tr!("tool-empty-output")
                        } else {
                            crate::i18n::tr!("tool-awaiting-output")
                        },
                        cx,
                    )
                })),
            ]
        }.into_any_element()
    }
    fn tool_has_rendered_output(&self, tool: &ToolCall) -> bool {
        tool.content.iter().any(|content| match content {
            acp::ToolCallContent::Content(_) | acp::ToolCallContent::Diff(_) => true,
            acp::ToolCallContent::Terminal(terminal) => self
                .selected
                .timeline
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
            .selected
            .timeline
            .conversation
            .as_ref()
            .and_then(|c| c.terminals.get(reference.terminal_id.0.as_ref()));
        if let Some(terminal) = terminal {
            gds_uim::el! {
                #div {
                    id: $SharedString::from(format!(
                    "terminal-{}",
                    reference.terminal_id
                    ));
                    test_support; flex; flex_col; gap_2;
                } [
                    ..$((!terminal.output.is_empty()).then(|| self.markdown(
                        format!("{id}-terminal-{}", reference.terminal_id),
                        code_block("text", &terminal.output),
                        index,
                        cx,
                    ))),
                    ..$terminal.exit.as_ref().map(|exit| {
                    let mut status = Vec::new();
                    if let Some(code) = exit.exit_code {
                        status.push(crate::i18n::tr!(
                            "terminal-exit-code",
                            code = (code).to_string()
                        ));
                    }
                    if let Some(signal) = &exit.signal {
                        status.push(crate::i18n::tr!(
                            "terminal-signal",
                            signal = (signal).to_string()
                        ));
                    }
                    gds_uim::el! {
                        #div {
                            id: $SharedString::from(format!(
                                "terminal-exit-{}",
                                reference.terminal_id
                            ));
                            test_support;
                        } [
                            $caption(
                                if status.is_empty() {
                                    crate::i18n::tr!("terminal-exited").to_owned()
                                } else {
                                    status.join(" · ")
                                },
                                cx,
                            ),
                        ]
                    }
                    }),
                    ..$((terminal.output.is_empty()
                        && terminal.exit.is_none()
                        && tool.raw_output.is_none()).then(|| {
                        caption(
                            if matches!(
                                tool.status,
                                ToolCallStatus::Completed | ToolCallStatus::Failed
                            ) {
                                crate::i18n::tr!("terminal-empty-output")
                            } else {
                                crate::i18n::tr!("terminal-awaiting-output")
                            },
                            cx,
                        )
                    })),
                ]
            }
            .into_any_element()
        } else if tool.raw_output.is_some() {
            gds_uim::el! { #div }.into_any_element()
        } else {
            caption(
                if matches!(
                    tool.status,
                    ToolCallStatus::Completed | ToolCallStatus::Failed
                ) {
                    crate::i18n::tr!("terminal-empty-output")
                } else {
                    crate::i18n::tr!("terminal-awaiting-output")
                },
                cx,
            )
            .into_any_element()
        }
    }
}

fn copy_chat_text(text: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
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

pub(in crate::workspace) fn block_source(block: &ContentBlock) -> Option<String> {
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

pub(in crate::workspace) fn render_content(
    id: String,
    block: &ContentBlock,
    markdown: Option<AnyElement>,
    images: &Entity<images::Cache>,
    audio_player: &Entity<crate::audio::Player>,
    local_files: bool,
    cx: &App,
) -> AnyElement {
    match block {
        ContentBlock::Text(_) => gds_uim::el! { #div [..markdown] }.into_any_element(),
        ContentBlock::ResourceLink(resource) => super::resource::render(&id, resource, local_files, cx),
        ContentBlock::Image(image) => gds_uim::el! {
            #div { flex; flex_col; gap_2; } [
                $caption(crate::i18n::tr!("resource-image", type = (image.mime_type).to_string()), cx),
                $images::acp_image(&id, image, images, false, cx),
            ]
        }.into_any_element(),
        ContentBlock::Resource(resource) => match &resource.resource {
            acp::EmbeddedResourceResource::TextResourceContents(text) => gds_uim::el! {
                #div { flex; flex_col; gap_2; } [$caption(text.uri.clone(), cx), ..markdown]
            }.into_any_element(),
            acp::EmbeddedResourceResource::BlobResourceContents(blob) => caption(
                crate::i18n::tr!("resource-file", type = (blob.uri).to_string(), uri = (blob.mime_type
                        .as_deref()
                        .unwrap_or("application/octet-stream")).to_string()),
                cx,
            )
            .into_any_element(),
            _ => caption(crate::i18n::tr!("resource-embedded"), cx).into_any_element(),
        },
        ContentBlock::Audio(audio) => {
            crate::audio::content(&id, &id, audio, audio_player, cx)
        }
        _ => caption(crate::i18n::tr!("resource-preview-unavailable"), cx).into_any_element(),
    }
}
