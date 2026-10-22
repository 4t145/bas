//! Deterministic desktop fixtures; available only in the GUI testing build.

use super::*;
use bas::{
    acp,
    model::{Cursor, Event, EventKind, Owner, StopReason, TurnId},
};
use base64::Engine as _;
use tokio::sync::mpsc;

impl Workspace {
    pub(crate) fn preview(
        fixture: &str,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let interaction = fixture;
        let fixture = match fixture {
            "agent-hover" | "attachment-hint" => "usage",
            "app-menu" | "thread-menu" => "tool-details",
            "markdown-copy-hover" | "subtle-actions" => "markdown-actions",
            "turn-navigation-hover" => "turn-navigation",
            "activity-artifacts-collapsed" => "activity-artifacts",
            _ => fixture,
        };
        let (commands, mut requests) = mpsc::unbounded_channel();
        let (updates, events) = mpsc::channel(EVENT_BATCH);
        let mut view = Self::with_connection(
            Connection::for_test(commands),
            events,
            "127.0.0.1:0".parse().expect("preview address"),
            PathBuf::from("/project"),
            compact,
            window,
            cx,
        );
        // Keep the fixture interactive without issuing network requests.
        view.backend._events = cx.spawn(async move |_, _| {
            let _updates = updates;
            while requests.recv().await.is_some() {}
        });
        let mut session = SessionMeta::new(SessionId::new(), "preview".into(), Owner::current());
        session.status = SessionStatus::Idle;
        session.title = Some("Markdown 排版".into());
        session.cwd = Some(PathBuf::from("/project"));
        if fixture == "startup" {
            session.agent = "pi".into();
            session.title = None;
        }
        if fixture == "agent-menu" {
            session.agent = "embedded".into();
            session.title = None;
        }
        if fixture == "images" {
            session.title = Some("图片显示".into());
        }
        if fixture == "activity-artifacts" {
            session.title = Some("活动与图片展示".into());
        }
        session.acp = Some(
            acp::NewSessionResponse::new(session.id.to_string()).config_options(vec![
                acp::SessionConfigOption::select(
                    "model",
                    "模型",
                    "default",
                    if fixture == "menus" {
                        (0..24)
                            .map(|i| {
                                acp::SessionConfigSelectOption::new(
                                    format!("model-{i}"),
                                    format!("Model {i} · 中文选项"),
                                )
                            })
                            .collect()
                    } else {
                        vec![acp::SessionConfigSelectOption::new("default", "Default")]
                    },
                ),
                acp::SessionConfigOption::select(
                    "thinking",
                    "Thinking",
                    "off",
                    vec![acp::SessionConfigSelectOption::new("off", "Thinking: off")],
                ),
            ]),
        );
        session.initialization = Some(
            acp::InitializeResponse::new(agent_client_protocol::schema::ProtocolVersion::V1)
                .agent_capabilities(
                    acp::AgentCapabilities::new().prompt_capabilities(
                        acp::PromptCapabilities::new()
                            .image(true)
                            .embedded_context(true),
                    ),
                ),
        );
        view.selected.id = Some(session.id);
        view.selected.agent = "preview".into();
        view.sessions.entries = vec![session.clone()];
        view.backend.agents = vec![connection::AgentSummary {
            id: "preview".into(),
            kind: "preview".into(),
            command: String::new(),
        }];
        if fixture == "agent-menu" {
            view.selected.agent = session.agent.to_string();
            view.backend.agents = ["codex", "codex-acp", "embedded", "pi"]
                .into_iter()
                .map(|id| connection::AgentSummary {
                    id: id.into(),
                    kind: "preview".into(),
                    command: String::new(),
                })
                .collect();
        }
        if fixture == "menus" {
            view.backend
                .agents
                .extend((0..24).map(|i| connection::AgentSummary {
                    id: format!("Agent {i} · 中文后端"),
                    kind: "preview".into(),
                    command: String::new(),
                }));
        }
        view.backend.connected = true;
        view.selected.creating = false;
        view.selected.stream = StreamState::Live;
        view.selected.error = None;
        let mut conversation = Conversation::new(session.clone());
        if fixture == "activity-artifacts" {
            let image = acp::ImageContent::new(
                base64::engine::general_purpose::STANDARD
                    .encode(include_bytes!("images/assets/sample.png")),
                "image/png",
            );
            for (index, update) in [
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                    "检查生成图片与思考、工具行的布局。".into(),
                )),
                acp::SessionUpdate::ToolCall(
                    acp::ToolCall::new("generated", "Image generation")
                        .status(acp::ToolCallStatus::Completed)
                        .content(vec![acp::ToolCallContent::Content(acp::Content::new(
                            acp::ContentBlock::Image(image),
                        ))]),
                ),
                acp::SessionUpdate::AgentThoughtChunk(
                    acp::ContentChunk::new("检查图片。".into()).message_id("first"),
                ),
                acp::SessionUpdate::AgentThoughtChunk(
                    acp::ContentChunk::new("准备结果。".into()).message_id("second"),
                ),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    "图片保留在工具调用结果中，展开工具输出即可查看。".into(),
                )),
            ]
            .into_iter()
            .enumerate()
            {
                conversation.apply(Event::new(
                    session.id,
                    Cursor::from_raw(index as u64 + 1),
                    EventKind::Acp(update),
                ));
            }
            if interaction == "activity-artifacts" {
                view.selected
                    .timeline
                    .expanded
                    .insert("activity-tool-generated".into());
            }
        }
        if matches!(fixture, "turn-navigation" | "turn-navigation-many") {
            navigation_turns(
                &mut conversation,
                if fixture == "turn-navigation-many" {
                    80
                } else {
                    3
                },
            );
        }
        if fixture == "markdown-actions" {
            for (index, update) in [
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                    "请展示代码片段与表格。".into(),
                )),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    include_str!("markdown/assets/copy-actions.md").into(),
                )),
            ]
            .into_iter()
            .enumerate()
            {
                conversation.apply(Event::new(
                    session.id,
                    Cursor::from_raw(index as u64 + 1),
                    EventKind::Acp(update),
                ));
            }
        }
        if matches!(fixture, "usage" | "usage-details" | "usage-hover") {
            conversation.apply(Event::new(
                session.id,
                Cursor::from_raw(1),
                EventKind::Acp(acp::SessionUpdate::UsageUpdate(
                    acp::UsageUpdate::new(109_000, 258_000).cost(acp::Cost::new(0.035, "USD")),
                )),
            ));
        }
        if fixture == "startup" {
            conversation.apply(Event::new(
                session.id,
                Cursor::from_raw(1),
                EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
                    acp::ContentChunk::new("**pi v0.87.1**\n\nAgent 已准备好。".into()),
                )),
            ));
        }
        if fixture == "files" {
            let file = acp::TextResourceContents::new(
                "# 全机 SSH 公钥清单 —— 注释保持原样\n# modules/nixos/ssh.nix\n{ atlas = \"ssh-ed25519 test\"; }",
                "file:///project/keys.nix",
            ).mime_type("text/x-nix");
            for (index, content) in [
                acp::ContentBlock::Resource(acp::EmbeddedResource::new(
                    acp::EmbeddedResourceResource::TextResourceContents(file.clone()),
                )),
                acp::ContentBlock::from(format!(
                    "普通正文与 `inline code`\n\n{}",
                    crate::attachment::file_code(&file.text, &file.uri)
                )),
            ]
            .into_iter()
            .enumerate()
            {
                conversation.apply(Event::new(
                    session.id,
                    Cursor::from_raw(index as u64 + 1),
                    EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
                        acp::ContentChunk::new(content),
                    )),
                ));
            }
            view.composer.input.update(cx, |input, cx| {
                input.set_value("输入文字与下方按钮对齐", window, cx)
            });
        }
        if fixture == "images" {
            for (index, kind) in image_events().into_iter().enumerate() {
                conversation.apply(Event::new(
                    session.id,
                    Cursor::from_raw(index as u64 + 1),
                    kind,
                ));
            }
        }
        if matches!(fixture, "sample" | "layout" | "inline-wrap") {
            let updates = if fixture == "layout" {
                vec![
                    acp::SessionUpdate::ToolCall(
                        acp::ToolCall::new("short", "pwd && ls -la")
                            .status(acp::ToolCallStatus::Completed),
                    ),
                    acp::SessionUpdate::ToolCall(
                        acp::ToolCall::new("long", "echo '--- 已装工具 ---'; for c in git rg fd fzf jq curl htop nvim tmux neofetch; do command -v $c; done")
                            .status(acp::ToolCallStatus::Completed),
                    ),
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                        include_str!("assets/layout-preview.md").to_owned().into(),
                    )),
                ]
            } else {
                vec![
                    acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                        "请展示 Markdown：中文、English、日本語、한국어，以及 `行内代码`。".into(),
                    )),
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                        (if fixture == "inline-wrap" {
                            include_str!("markdown/assets/inline-wrap.md")
                        } else {
                            include_str!("assets/panel-preview.md")
                        })
                        .to_owned()
                        .into(),
                    )),
                ]
            };
            for (index, update) in updates.into_iter().enumerate() {
                conversation.apply(Event::new(
                    session.id,
                    Cursor::from_raw(index as u64 + 1),
                    EventKind::Acp(update),
                ));
            }
        }
        if matches!(fixture, "thoughts" | "completed" | "reply-preview") {
            let turn = TurnId::new();
            for (index, kind) in [
                EventKind::TurnStarted { turn },
                EventKind::Acp(acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
                    "Planning harmless terminal commands\n\n检查环境与工作目录，再运行只读命令。".into(),
                ))),
                EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    "已完成环境检查，所有命令执行成功。\n\n中文、English、日本語、한국어，**粗体 Bold** 和 `行内代码 inline code`。\n\n- 工作目录：`/project`\n- 状态：一切正常 ✅\n\n```sh\npwd && ls -la\n```\n\n[查看文档](https://example.com)".into(),
                ))),
                EventKind::TurnFinished { turn, result: Ok(acp::PromptResponse::new(StopReason::EndTurn)) },
            ].into_iter().enumerate() {
                conversation.apply(Event::new(session.id, Cursor::from_raw(index as u64 + 1), kind));
            }
            view.selected
                .timeline
                .expanded
                .insert(conversation.items[0].id());
        }
        if fixture == "tool-details" {
            let command = "node /project/long-task.mjs";
            for (index, update) in [
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                    "检查文档并运行长任务，优先查看执行结果。".into(),
                )),
                acp::SessionUpdate::ToolCall(
                    acp::ToolCall::new("search", "WebSearch for hooks")
                        .kind(acp::ToolKind::Search)
                        .status(acp::ToolCallStatus::Completed)
                        .raw_input(serde_json::json!({
                            "type": "webSearch",
                            "query": "https://learn.chatgpt.com/docs/hooks",
                            "action": {"type": "openPage"}
                        }))
                        .content(vec![acp::ToolCallContent::Content(acp::Content::new(
                            "找到文档：\n\n- Hooks 概览\n- 配置示例\n- 生命周期说明",
                        ))]),
                ),
                acp::SessionUpdate::ToolCall(
                    acp::ToolCall::new("long-task", "Running long task in /project")
                        .kind(acp::ToolKind::Execute)
                        .status(acp::ToolCallStatus::InProgress)
                        .raw_input(serde_json::json!({
                            "command": command,
                            "script": "// 长任务实现\n".repeat(40)
                        }))
                        .raw_output(serde_json::json!(
                            "开始采样\n\n[8.4%] 已运行 10.0 秒，采样 38,200,000 次\n[16.7%] 已运行 20.0 秒，采样 76,000,000 次\n[25.0%] 已运行 30.0 秒，π ≈ 3.1416199\n\n结果目录：/project/output"
                        )),
                ),
            ]
            .into_iter()
            .enumerate()
            {
                conversation.apply(Event::new(
                    session.id,
                    Cursor::from_raw(index as u64 + 1),
                    EventKind::Acp(update),
                ));
            }
            for item in &conversation.items {
                if matches!(item, Item::Tool(_)) {
                    view.selected.timeline.expanded.insert(item.id());
                }
            }
            view.selected
                .timeline
                .expanded
                .insert("activity-tool-search".into());
        }
        let mut hover_attachment = None;
        if matches!(fixture, "attachments" | "attachment-preview") {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("请查看这张图 ", window, cx);
                let end = input.value().len();
                input.set_selected_range(end..end, cx);
            });
            let image = gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Svg,
                include_bytes!("assets/attachment-preview.svg").to_vec(),
            );
            let attachment = Attachment::image(&image).expect("preview image");
            hover_attachment = Some(format!(
                "attachment-{}-{}",
                attachment.id,
                view.composer.input.read(cx).value().len()
            ));
            view.add_attachments(session.id, vec![attachment], window, cx);
            view.composer.input.update(cx, |input, cx| {
                input.replace(" 并说明图片中的内容。", window, cx)
            });
            conversation.session.status = SessionStatus::Working;
        }
        if matches!(fixture, "multiline" | "multiline-hover") {
            let command = "/run/current-system/sw/bin/bash -lc 'bash /tmp/zed-delta-install.ZVCcme/Delta/install.sh\n/home/atlas/.local/bin/delta --version\ncat ~/.local/share/applications/dev.zed.Delta.desktop'";
            conversation.apply(Event::new(
                session.id,
                Cursor::from_raw(1),
                EventKind::Acp(acp::SessionUpdate::ToolCall(
                    acp::ToolCall::new("multiline", command).status(acp::ToolCallStatus::Completed),
                )),
            ));
            if fixture == "multiline" {
                view.selected
                    .timeline
                    .expanded
                    .insert("tool-multiline".into());
            }
        }
        if fixture == "thread-feedback" {
            for (index, update) in feedback_updates().into_iter().enumerate() {
                let cursor = if matches!(update, acp::SessionUpdate::Notice(_)) {
                    Cursor::START
                } else {
                    Cursor::from_raw(index as u64 + 1)
                };
                conversation.apply(Event::new(session.id, cursor, EventKind::Acp(update)));
            }
            let first = conversation.items[1].id();
            view.selected
                .timeline
                .expanded
                .insert(format!("activity-{first}"));
        }
        view.selected.timeline.list.reset(conversation.rows.len());
        view.selected
            .timeline
            .list
            .set_follow_mode(FollowMode::Normal);
        view.selected
            .timeline
            .list
            .scroll_to(gpui_kit::ListOffset::default());
        view.composer.expanded = !compact && conversation.items.is_empty();
        if matches!(fixture, "empty" | "startup" | "agent-menu") {
            view.sessions.unstarted.insert(session.id);
            view.composer.expanded = !compact;
        }
        view.sync_composer_mode(cx);
        if matches!(fixture, "turn-navigation" | "turn-navigation-many") {
            let entries = conversation
                .items
                .iter()
                .filter_map(|item| {
                    let Item::Message { message, .. } = item else {
                        return None;
                    };
                    if message.role != bas::model::Role::User {
                        return None;
                    }
                    Some(bas::service::HistoryOverviewEntry {
                        cursor: turn_navigation::message_cursor(message)?.saturating_sub(1),
                        timestamp_millis: message.at.millis() as i64,
                        preview: message
                            .content
                            .iter()
                            .filter_map(|block| {
                                if let bas::model::ContentBlock::Text(text) = block {
                                    Some(text.text.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<String>(),
                    })
                })
                .collect();
            view.selected.timeline.navigation.overview = Some(
                bas::service::HistoryOverview {
                    session: session.id,
                    cursor: conversation.cursor,
                    entries,
                }
                .into(),
            );
        }
        view.selected.timeline.conversation = Some(conversation);
        view.window.output_open = matches!(
            fixture,
            "sample"
                | "layout"
                | "files"
                | "images"
                | "thoughts"
                | "multiline"
                | "multiline-hover"
                | "tool-details"
                | "activity-artifacts"
                | "thread-feedback"
                | "markdown-actions"
                | "turn-navigation"
                | "turn-navigation-many"
        );
        if matches!(fixture, "connecting" | "disconnected") {
            view.backend.connected = false;
            view.selected.stream = StreamState::Loading;
            view.backend.error = (fixture == "disconnected").then(|| "预览：服务不可用".into());
            if fixture == "connecting" {
                view.selected.detach();
                view.composer.expanded = true;
            }
            view.composer.input.update(cx, |input, cx| {
                input.set_value("连接恢复后发送这份草稿", window, cx)
            });
        }
        view.resize_window(window, cx);
        if fixture == "clipboard" {
            cx.spawn_in(window, async move |view, cx| {
                use gpui_kit::test::TestWindowExt;
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(700))
                    .await;
                let _ = cx.update(|window, cx| window.press("ctrl-v", cx));
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                let _ = view.update(cx, |view, _| {
                    println!(
                        "clipboard attachment count: {}",
                        view.composer.attachments.len()
                    );
                    println!(
                        "clipboard attachment errors: {}",
                        view.selected.error.is_some()
                    );
                });
            })
            .detach();
        }
        let auto_open = match interaction {
            "app-menu" => Some("app-menu"),
            "menus" => Some("config-model"),
            "agent-menu" => Some("agent-picker"),
            "picker" => Some("add-attachment"),
            "usage-details" => Some("session-usage"),
            _ => None,
        };
        if let Some(button) = auto_open {
            let hover_menu = fixture == "agent-menu" && !compact;
            cx.spawn_in(window, async move |_, cx| {
                use gpui_kit::test::TestWindowExt;
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let _ = cx.update(|window, cx| {
                    window.click(button, cx);
                    if hover_menu {
                        window.render_frame(cx);
                        window.hover("popup-menu", cx);
                    }
                });
            })
            .detach();
        }
        if interaction == "thread-menu" {
            cx.spawn_in(window, async move |view, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(700))
                    .await;
                use gpui_kit::test::TestWindowExt;
                let session = view
                    .read_with(cx, |view, _| view.selected.id.expect("preview session"))
                    .expect("preview view");
                let _ = cx.update(|window, cx| {
                    window.right_click(SharedString::from(format!("session-{session}")), cx)
                });
            })
            .detach();
        }
        if fixture == "markdown-actions" {
            let interaction = interaction.to_owned();
            let reply = view
                .selected
                .timeline
                .conversation
                .as_ref()
                .expect("preview conversation")
                .items
                .last()
                .expect("preview reply")
                .id();
            cx.spawn_in(window, async move |_, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(700))
                    .await;
                use gpui_kit::test::TestWindowExt;
                let start = include_str!("markdown/assets/copy-actions.md")
                    .match_indices("```bash")
                    .nth(1)
                    .expect("second code block")
                    .0;
                let _ = cx.update(|window, cx| {
                    if interaction == "subtle-actions" {
                        window.hover(SharedString::from(format!("message-actions-{reply}")), cx);
                    } else {
                        window.hover(("markdown-code-block", start), cx);
                        if interaction == "markdown-copy-hover" {
                            window
                                .within(("markdown-code-block", start))
                                .hover("copy-markdown-code", cx);
                        }
                    }
                });
            })
            .detach();
        }
        let hover = match interaction {
            "agent-hover" => Some("agent-picker".to_owned()),
            "attachment-hint" => Some("add-attachment".to_owned()),
            "reply-preview" => Some("completed-reply".to_owned()),
            "attachment-preview" => hover_attachment,
            "multiline-hover" => Some("tool-command-tool-multiline".to_owned()),
            "usage-hover" => Some("session-usage".to_owned()),
            "turn-navigation-hover" => Some("turn-marker-1".to_owned()),
            _ => None,
        };
        if let Some(hover) = hover {
            cx.spawn_in(window, async move |_, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(700))
                    .await;
                use gpui_kit::test::TestWindowExt;
                let _ = cx.update(|window, cx| window.hover(SharedString::from(hover), cx));
            })
            .detach();
        }
        view
    }
}

fn navigation_turns(conversation: &mut Conversation, count: usize) {
    let session = conversation.session.id;
    let mut cursor = 0;
    for number in 1..=count {
        let turn = TurnId::new();
        for kind in [
            EventKind::TurnStarted { turn },
            EventKind::Acp(acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                format!("第 {number} 轮：检查会话布局与快捷跳转。").into(),
            ))),
            EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                if number == 2 {
                    "这是一段较长的回复。\n\n".repeat(40)
                } else {
                    "已检查消息卡片、工具详情和侧边快捷入口。\n\n卡片左右留白一致，工具详情先展示输入，再展示输出。".into()
                }
                .into(),
            ))),
            EventKind::TurnFinished {
                turn,
                result: Ok(acp::PromptResponse::new(StopReason::EndTurn)),
            },
        ] {
            cursor += 1;
            conversation.apply(Event::new(session, Cursor::from_raw(cursor), kind));
        }
    }
}

fn image_events() -> Vec<EventKind> {
    let image_path = std::env::var_os("BAS_PREVIEW_IMAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src/bin/basg/workspace/assets/attachment-preview.svg")
        });
    let sections = [
        vec![acp::ContentBlock::from(
            "这里分别展示 Markdown 本地图片与 ACP 图片。含空格的本地路径使用尖括号包住图片目标。\n\n连续回复段落共用线程中的 Agent 信息，末尾操作按钮悬停时应变亮。",
        )],
        vec![acp::ContentBlock::from(format!(
            "### Markdown 本地图片\n\n![本地图片](<{}>)",
            image_path.display()
        ))],
        vec![
            acp::ContentBlock::from("### ACP 图片\n\n下面是直接传入的 SVG 图片内容。\n\n"),
            acp::ContentBlock::Image(acp::ImageContent::new(
                base64::engine::general_purpose::STANDARD
                    .encode(include_bytes!("assets/attachment-preview.svg")),
                "image/svg+xml",
            )),
            acp::ContentBlock::from(
                "\n\n可以滚动查看两种图片，悬停复制回复与返回用户消息按钮，检查图标高亮效果。",
            ),
        ],
    ];
    let mut events = Vec::new();
    for (index, section) in sections.into_iter().enumerate() {
        let turn = TurnId::new();
        events.push(EventKind::TurnStarted { turn });
        if index == 0 {
            events.push(EventKind::Acp(acp::SessionUpdate::UserMessageChunk(
                acp::ContentChunk::new("请展示本地 Markdown 图片和 ACP 图片。".into()),
            )));
        }
        events.extend(section.into_iter().map(|content| {
            EventKind::Acp(acp::SessionUpdate::AgentMessageChunk(
                acp::ContentChunk::new(content),
            ))
        }));
        events.push(EventKind::TurnFinished {
            turn,
            result: Ok(acp::PromptResponse::new(StopReason::EndTurn)),
        });
    }
    events
}

fn feedback_updates() -> Vec<acp::SessionUpdate> {
    use acp::*;
    vec![
        SessionUpdate::UserMessageChunk(ContentChunk::new(
            "检查工作区配置，并核对相关文档。".into(),
        )),
        SessionUpdate::AgentThoughtChunk(ContentChunk::new(
            "先检查本地文件，再核对外部文档。".into(),
        )),
        SessionUpdate::ToolCall(
            ToolCall::new("config", "读取工作区配置")
                .kind(ToolKind::Read)
                .status(ToolCallStatus::Completed)
                .raw_input(serde_json::json!({"path": "/project/config.toml"}))
                .raw_output(serde_json::json!("配置已读取")),
        ),
        SessionUpdate::ToolCall(
            ToolCall::new("docs", "检索文档")
                .kind(ToolKind::Fetch)
                .status(ToolCallStatus::Failed)
                .raw_input(serde_json::json!({"url": "https://example.com/docs"}))
                .raw_output(serde_json::json!("连接超时")),
        ),
        SessionUpdate::CompactionUpdate(
            CompactionUpdate::new("summary", CompactionStatus::Completed)
                .summary(vec!["已保留配置检查结果与待核对的文档地址。".into()]),
        ),
        SessionUpdate::AgentMessageChunk(ContentChunk::new(
            "配置检查已完成。文档服务暂时不可用，可以稍后重试。".into(),
        )),
        SessionUpdate::PlanUpdate(PlanUpdate::new(PlanUpdateContent::items(
            "review",
            vec![
                PlanEntry::new(
                    "检查工作区配置",
                    PlanEntryPriority::High,
                    PlanEntryStatus::Completed,
                ),
                PlanEntry::new(
                    "核对文档",
                    PlanEntryPriority::Medium,
                    PlanEntryStatus::Pending,
                ),
            ],
        ))),
        SessionUpdate::Notice(
            Notice::new(NoticeSeverity::Warning, "文档服务暂时不可用")
                .description("本地检查结果已保留。"),
        ),
        SessionUpdate::UsageUpdate(UsageUpdate::new(42_000, 200_000)),
    ]
}
