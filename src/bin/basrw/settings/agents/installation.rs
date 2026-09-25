//! Documentation only: these examples never create specs, install packages or run commands.

use gpui_kit::component::*;
use gpui_kit::{App, IntoElement, ParentElement, Styled, TestSupportExt, div, prelude::*};

struct Guide {
    id: &'static str,
    title: &'static str,
    url: &'static str,
    install: &'static str,
    command: &'static str,
    setup: &'static str,
}

const GUIDES: &[Guide] = &[
    Guide {
        id: "codex",
        title: "Codex ACP",
        url: "https://github.com/agentclientprotocol/codex-acp#installation",
        install: "npm install -g @agentclientprotocol/codex-acp",
        command: "程序：codex-acp 的绝对路径；参数：[]",
        setup: "按适配器文档登录。它如何选择 Codex 由适配器负责；BASR 不指定 CLI 版本或覆盖 CODEX_PATH。",
    },
    Guide {
        id: "opencode",
        title: "OpenCode",
        url: "https://opencode.ai/docs/acp/",
        install: "按官方文档选择适合系统的 OpenCode 安装方式。",
        command: "程序：opencode 的绝对路径；参数：[\"acp\"]",
        setup: "先在 OpenCode 中配置模型提供方并完成登录，再通过 ACP 连接。",
    },
    Guide {
        id: "pi",
        title: "Pi ACP",
        url: "https://github.com/svkozak/pi-acp#install",
        install: "npm install -g pi-acp",
        command: "程序：pi-acp 的绝对路径；参数：[]",
        setup: "按适配器 Prerequisites 安装 Pi、配置提供方；可在终端运行 pi-acp --terminal-login。",
    },
];

pub(super) fn render(cx: &App) -> impl IntoElement {
    div()
        .id("agents-installation-guide")
        .test_support()
        .flex()
        .flex_col()
        .gap_4()
        .text_sm()
        .child("BASR 只连接 ACP 程序，不安装、升级或固定第三方 Agent。下面是手动安装示例，具体要求以官方文档为准。")
        .children(GUIDES.iter().map(|guide| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .pt_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(guide.title)
                        .child(
                            super::button(format!("agents-guide-{}", guide.id))
                                .label("安装 / 登录文档")
                                .on_click(move |_, _, cx| cx.open_url(guide.url)),
                        ),
                )
                .child(guide.install)
                .child(guide.command)
                .child(
                    div().text_color(cx.theme().muted_foreground).child(guide.setup),
                )
        }))
        .child(
            div()
                .border_t_1()
                .border_color(cx.theme().border)
                .pt_3()
                .flex()
                .flex_col()
                .gap_2()
                .child("其他 ACP 程序同样适用：按其文档安装与登录，然后填写可执行文件的路径和 JSON 参数数组。")
                .child("Linux / macOS 可用 command -v 程序名 查找路径。basrd 的 PATH 可能与终端不同，建议填写绝对路径；不要把整条命令填进程序路径。")
                .child("环境变量继承自 basrd 服务，也可通过 settings.json 的 daemon.agents[].env 配置。不要在共享配置中存放 API key；优先使用工具自己的登录流程。")
                .child("旧版迁移：以前由 BASR 安装的适配器和现有配置不会被卸载或改写。若仍指向旧包，请安装并确认自己的 ACP 程序路径，再修改配置、连接验证；不要只更新另一个同名 CLI。"),
        )
}
