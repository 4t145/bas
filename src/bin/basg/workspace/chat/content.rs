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
                el! {
                    .div {
                        # format!("message-surface-{id}").SharedString::from;
                        test_support; w_full; min_w_0;
                        group: ~group;
                        relative; flex; flex_col; gap_2; px_3; py_2; rounded_md;
                        @if (user)
                            border_1;
                            border_color: $cx.theme().border;
                            bg: $cx.theme().muted.opacity(0.35);
                        @end;
                    } [
                        ..$message
                            .content
                            .iter()
                            .enumerate()
                            .map(|(block_index, block)| {
                                self.render_block(message_block_id(message, block_index), block, index, cx)
                            }),
                        @if (user)
                            .div {
                                absolute; top_1; right_1; invisible;
                                group_hover: group, { visible; };
                            } [
                                (format!("copy-{id}").SharedString::from, IconName::Copy, cx).message_actions::action_button {
                                    accessibility_label: tr!("action-copy-message");
                                    tooltip: tr!("action-copy-message");
                                    on_click: move (_, _, cx) => ($&copy, cx).copy_chat_text;
                                },
                            ],
                        @end,
                    ]
                }
                .into_any_element()
            }
            Item::Tool(tool) => self.render_tool(&id, tool, index, cx),
            Item::Compaction(compaction) => self.render_compaction(&id, compaction, index, cx),
            Item::Notice { text, .. } => el! {
                .div { px_3; py_2; flex; items_start; gap_2; } [
                    IconName::Info.Icon::new {
                        size: ui::ICON.px;
                        text_color: $cx.theme().muted_foreground;
                        flex_shrink_0;
                    },
                    (~text, cx).caption { flex_1; min_w_0; },
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
            &self.link_context(),
            cx,
        )
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
            tool.name.as_deref().unwrap_or(tr!("tool-call"))
        } else {
            &tool.title
        };
        let (label, status_icon, color) = match tool.status {
            ToolCallStatus::Pending => (
                tr!("status-pending"),
                IconName::Clock,
                cx.theme().muted_foreground,
            ),
            ToolCallStatus::InProgress => {
                (tr!("status-running"), IconName::Loader, cx.theme().primary)
            }
            ToolCallStatus::Completed => {
                (tr!("status-completed"), IconName::Check, cx.theme().success)
            }
            ToolCallStatus::Failed => (tr!("status-failed"), IconName::CircleX, cx.theme().danger),
            _ => (
                tr!("status-unknown"),
                IconName::CircleAlert,
                cx.theme().muted_foreground,
            ),
        };
        let command = el! {
            .div {
                # format!("tool-command-{id}").SharedString::from;
                test_support; flex_1; min_w_0; overflow_hidden;
            } [
                $(self.disclosure(
                    id,
                    &title.split_whitespace().collect::<Vec<_>>().join(" "),
                    expanded,
                    index,
                    Some(status::activity_icon(
                        format!("tool-kind-{id}"),
                        tool_icon(tool.kind),
                        if tool.kind == bas::model::ToolKind::Delete { cx.theme().danger } else { cx.theme().muted_foreground },
                        false,
                    )),
                    cx,
                )) { w_full; },
            ]
        };
        let status = el! {
            format!("status-{id}").SharedString::from.Button::new {
                ghost; xsmall; cursor_default;
                w: ui::COMPACT_CONTROL.px; h: ui::COMPACT_CONTROL.px;
                p_0; flex_none; tooltip: label;
            } [
                $status::activity_icon(
                    SharedString::from(format!("tool-status-animation-{id}")),
                    status_icon, color, tool.status == ToolCallStatus::InProgress,
                ),
            ]
        }
        .into_any_element();
        let header = self.content_header(
            id,
            ui::tooltip::bounded(
                SharedString::from(format!("tool-hint-{id}")),
                command,
                title.to_owned(),
            ),
            Some(status),
        );
        el! {
            (true, cx).content_header::frame [
                header,
                @if (expanded)
                    .div { flex; flex_col; gap_3; px_3; pt_1; pb_3; } [
                        @if (self.tool_has_input(tool))
                            .div {
                                # format!("tool-input-section-{id}").SharedString::from;
                                test_support; flex; flex_col; gap_2;
                            } [
                                $(self.disclosure(
                                    &input_key,
                                    tr!("tool-input"),
                                    input_open,
                                    index,
                                    None,
                                    cx,
                                )) { w_full; text_color: $cx.theme().muted_foreground; },
                                @if (input_open)
                                    $self.render_tool_input(id, tool, index, cx),
                                @end,
                            ],
                        @end,
                        .div {
                            # format!("tool-output-section-{id}").SharedString::from;
                            test_support; flex; flex_col; gap_2;
                        } [
                            $(self.disclosure(
                                &output_key,
                                tr!("tool-output"),
                                output_open,
                                index,
                                None,
                                cx,
                            )) { w_full; text_color: $cx.theme().muted_foreground; },
                            @if (output_open)
                                $self.render_tool_output(id, tool, index, cx),
                            @end,
                        ],
                        $self.render_content_actions(id, id, index, cx),
                    ],
                @end,
            ]
        }
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
        el! {
            .div {
                # format!("tool-input-{id}").SharedString::from;
                test_support; flex; flex_col; gap_2; px_3; pb_3;
            } [
                @if (let Some(name) = tool.name.as_ref())
                    .div {
                        # format!("tool-name-{id}").SharedString::from;
                        test_support;
                    } [
                        (
                            tr!("tool-name", name = (name).to_string()),
                            cx,
                        ).caption,
                    ],
                @end,
                @if (!tool.raw_input.as_ref().is_some_and(|input| input_contains_command(input, &tool.title))
                    && !title_is_terminal_command
                    && (tool.title.contains(['\r', '\n']) || tool.title.chars().count() > 96))
                    $self.markdown(
                        format!("{id}-command"),
                        code_block("sh", &tool.title),
                        index,
                        cx,
                    ),
                @end,
                @if (let Some(input) = tool.raw_input.as_ref())
                    $self.render_tool_parameters(id, input, index, cx),
                @end,
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
        el! {
            .div { flex; flex_col; gap_2; } [
                (tr!("tool-command"), cx).caption,
                $self.markdown(format!("{id}-input-command"), code_block("sh", command), index, cx),
                @if (!parameters.is_empty())
                    (tr!("tool-arguments"), cx).caption,
                    $self.markdown(
                        format!("{id}-input"),
                        json_block(&serde_json::Value::Object(parameters)),
                        index,
                        cx,
                    ),
                @end,
            ]
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
        Some(el! {
            .div { flex; flex_col; gap_2; } [
                @if (let Some(label) = terminal.label.as_ref().filter(|label| {
                    !tool.raw_input.as_ref().is_some_and(|input| input_contains_command(input, label))
                }))
                    $self.markdown(
                        format!("{id}-terminal-command-{}", reference.terminal_id),
                        code_block("sh", label),
                        index,
                        cx,
                    ),
                @end,
                @if (let Some(cwd) = terminal.cwd.as_ref())
                    .div {
                        # format!("terminal-cwd-{}", reference.terminal_id).SharedString::from;
                        test_support;
                    } [
                        (
                            tr!("tool-directory", path = (cwd.display()).to_string()),
                            cx,
                        ).caption,
                    ],
                @end,
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
        el! {
            .div {
                # format!("tool-output-{id}").SharedString::from;
                test_support; flex; flex_col; gap_3; pl_3; border_l_2;
                border_color: $cx.theme().primary.opacity(0.55);
            } [
                @if (let Some(output) = tool.raw_output.as_ref().filter(|_| !self.tool_has_rendered_output(tool)))
                    $self.markdown(format!("{id}-output"), json_block(output), index, cx),
                @end,
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
                        _ => caption(tr!("tool-preview-unavailable"), cx)
                            .into_any_element(),
                    }),
                @if (tool.content.is_empty() && tool.raw_output.is_none())
                    (
                        @if (matches!(
                            tool.status,
                            ToolCallStatus::Completed | ToolCallStatus::Failed
                        ))
                            tr!("tool-empty-output")
                        @else
                            tr!("tool-awaiting-output")
                        @end,
                        cx,
                    ).caption,
                @end,
            ]
        }
        .into_any_element()
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
            el! {
                .div {
                    # format!(
                    "terminal-{}",
                    reference.terminal_id
                    ).SharedString::from;
                    test_support; flex; flex_col; gap_2;
                } [
                    @if (!terminal.output.is_empty())
                        $self.markdown(
                            format!("{id}-terminal-{}", reference.terminal_id),
                            code_block("text", &terminal.output),
                            index,
                            cx,
                        ),
                    @end,
                    ..$terminal.exit.as_ref().map(|exit| {
                    let mut status = Vec::new();
                    if let Some(code) = exit.exit_code {
                        status.push(tr!(
                            "terminal-exit-code",
                            code = (code).to_string()
                        ));
                    }
                    if let Some(signal) = &exit.signal {
                        status.push(tr!(
                            "terminal-signal",
                            signal = (signal).to_string()
                        ));
                    }
                    el! {
                        .div {
                            # format!(
                                "terminal-exit-{}",
                                reference.terminal_id
                            ).SharedString::from;
                            test_support;
                        } [
                            (
                                @if (status.is_empty())
                                    tr!("terminal-exited") { to_owned; }
                                @else
                                    $status.join(" · ")
                                @end,
                                cx,
                            ).caption,
                        ]
                    }
                    }),
                    @if (terminal.output.is_empty()
                        && terminal.exit.is_none()
                        && tool.raw_output.is_none())
                        (
                            @if (matches!(
                                tool.status,
                                ToolCallStatus::Completed | ToolCallStatus::Failed
                            ))
                                tr!("terminal-empty-output")
                            @else
                                tr!("terminal-awaiting-output")
                            @end,
                            cx,
                        ).caption,
                    @end,
                ]
            }
            .into_any_element()
        } else if tool.raw_output.is_some() {
            el! { .div }.into_any_element()
        } else {
            caption(
                if matches!(
                    tool.status,
                    ToolCallStatus::Completed | ToolCallStatus::Failed
                ) {
                    tr!("terminal-empty-output")
                } else {
                    tr!("terminal-awaiting-output")
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
    links: &links::Context,
    cx: &App,
) -> AnyElement {
    el! {
        @match (block)
        @case (ContentBlock::Text(_))
            .div [..markdown] { into_any_element; }
        @case (ContentBlock::ResourceLink(resource))
            ($&id, resource, links, cx).super::resource::render
        @case (ContentBlock::Image(image))
            .div { flex; flex_col; gap_2; } [
                (tr!("resource-image", type = (image.mime_type).to_string()), cx).caption,
                ($&id, image, images, false, cx).images::acp_image,
            ] { into_any_element; }
        @case (ContentBlock::Resource(resource))
            @match (&resource.resource)
            @case (acp::EmbeddedResourceResource::TextResourceContents(text))
                .div { flex; flex_col; gap_2; } [(~$(text.uri), cx).caption, ..markdown] { into_any_element; }
            @case (acp::EmbeddedResourceResource::BlobResourceContents(blob))
                (
                    tr!("resource-file", type = (blob.uri).to_string(), uri = (blob.mime_type
                        .as_deref()
                        .unwrap_or("application/octet-stream")).to_string()),
                    cx,
                ).caption { into_any_element; }
            @case (_)
                (tr!("resource-embedded"), cx).caption { into_any_element; }
            @end
        @case (ContentBlock::Audio(audio))
            ($&id, $&id, audio, audio_player, cx).crate::audio::content
        @case (_)
            (tr!("resource-preview-unavailable"), cx).caption { into_any_element; }
        @end
    }
}
