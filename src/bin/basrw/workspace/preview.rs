//! Deterministic desktop fixtures; available only in the GUI testing build.

use super::*;
use base64::Engine as _;
use basr::{
    acp,
    model::{Cursor, Event, EventKind, Owner, StopReason, TurnId},
};
use tokio::sync::mpsc;

impl Workspace {
    pub(crate) fn preview(
        fixture: &str,
        compact: bool,
        runtime: &tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        view._events = cx.spawn(async move |_, _| {
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
        view.active = Some(session.id);
        view.selected_agent = "preview".into();
        view.sessions = vec![session.clone()];
        view.agents = vec![pb::Agent {
            id: "preview".into(),
            kind: "preview".into(),
            command: String::new(),
        }];
        if fixture == "agent-menu" {
            view.selected_agent = session.agent.to_string();
            view.agents = ["codex", "codex-acp", "embedded", "pi"]
                .into_iter()
                .map(|id| pb::Agent {
                    id: id.into(),
                    kind: "preview".into(),
                    command: String::new(),
                })
                .collect();
        }
        if fixture == "menus" {
            view.agents.extend((0..24).map(|i| pb::Agent {
                id: format!("Agent {i} · 中文后端"),
                kind: "preview".into(),
                command: String::new(),
            }));
        }
        view.connected = true;
        view.creating = false;
        view.stream = StreamState::Live;
        view.error = None;
        let mut conversation = Conversation::new(session.clone());
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
            view.composer.update(cx, |input, cx| {
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
        if matches!(fixture, "sample" | "layout") {
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
                        include_str!("assets/panel-preview.md").to_owned().into(),
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
            view.expanded.insert(conversation.items[0].id());
        }
        if fixture == "tool-details" {
            let command = "node /project/long-task.mjs";
            for (index, update) in [
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                    "检查文档并运行长任务，优先查看执行结果。".into(),
                )),
                acp::SessionUpdate::ToolCall(
                    acp::ToolCall::new("search", "WebSearch for hooks")
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
                    view.expanded.insert(item.id());
                }
            }
            view.expanded.insert("activity-tool-search".into());
        }
        let mut hover_attachment = None;
        if matches!(fixture, "attachments" | "attachment-preview") {
            view.composer.update(cx, |input, cx| {
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
                view.composer.read(cx).value().len()
            ));
            view.add_attachments(session.id, vec![attachment], window, cx);
            view.composer.update(cx, |input, cx| {
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
                view.expanded.insert("tool-multiline".into());
            }
        }
        view.list.reset(conversation.rows.len());
        view.list.set_follow_mode(FollowMode::Normal);
        view.list.scroll_to(gpui_kit::ListOffset::default());
        view.composer_expanded = !compact && conversation.items.is_empty();
        if matches!(fixture, "empty" | "startup" | "agent-menu") {
            view.unstarted_sessions.insert(session.id);
            view.composer_expanded = !compact;
        }
        view.sync_composer_mode(cx);
        view.conversation = Some(conversation);
        view.output_open = matches!(
            fixture,
            "sample"
                | "layout"
                | "files"
                | "images"
                | "thoughts"
                | "multiline"
                | "multiline-hover"
                | "tool-details"
        );
        if matches!(fixture, "connecting" | "disconnected") {
            view.connected = false;
            view.stream = StreamState::Loading;
            view.connection_error = (fixture == "disconnected").then(|| "预览：服务不可用".into());
            view.composer.update(cx, |input, cx| {
                input.set_value("连接恢复后发送这份草稿", window, cx)
            });
        }
        view.placement = Some(desktop::Placement::new(runtime));
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
                    println!("clipboard attachment count: {}", view.attachments.len());
                    println!("clipboard attachment errors: {}", view.error.is_some());
                });
            })
            .detach();
        }
        let auto_open = match fixture {
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
        let hover = match fixture {
            "reply-preview" => Some("completed-reply".to_owned()),
            "attachment-preview" => hover_attachment,
            "multiline-hover" => Some("tool-command-tool-multiline".to_owned()),
            "usage-hover" => Some("session-usage".to_owned()),
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

fn image_events() -> Vec<EventKind> {
    let image_path = std::env::var_os("BASR_PREVIEW_IMAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src/bin/basrw/workspace/assets/attachment-preview.svg")
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
