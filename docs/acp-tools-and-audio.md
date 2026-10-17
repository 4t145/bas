# ACP 工具扩展、退出登录与音频

核对日期：2026-10-17。以下区分 ACP 标准、Agent 扩展和 Bas 自己的编排责任。

## 不只有 MCP：工具、skills、委托是不同层次

| 能力 | 作用 | ACP／Bas 的边界 |
| --- | --- | --- |
| MCP servers | 给 Agent 提供可调用的工具、资源和外部服务 | 稳定 ACP 通过会话生命周期请求中的 `mcpServers` 传入配置，Agent 负责连接。Bas 的统一配置与注入仍待接入。 |
| Skills | 可发现、按需加载的指令、工作流，以及相关脚本／资源 | 不是 MCP 工具的同义词。当前稳定 ACP 没有通用的 skills 安装、启用、删除接口；加载目录与扩展由 Agent 决定。发送一段 Markdown 上下文不等于安装了 skill。 |
| 委托工具 | 创建／选择另一个 Agent，派发工作，等待结果或继续通信 | 可由 Agent 原生实现，也可以由 Bas 编排层把已有会话 API 包装成 MCP 工具；需要另外定义身份、权限、工作目录、取消与结果关联，不能只增加一个协议标志。 |
| Subagent 展示 | 显示 Agent 已创建的子会话、消息、计划与状态 | [Subagents RFD](https://agentclientprotocol.com/rfds/subagents) 主要规范 Agent 创建的受限子会话；不等于给客户端提供任意创建、提示或控制子 Agent 的通用 API。 |

Skills 可以有 Agent 自有接口。例如核对的
[Qwen Code 快照](https://github.com/QwenLM/qwen-code/blob/720eff873a7fd32a0a94a1844f608a2696e26834/packages/cli/src/acp-integration/acpAgent.ts#L9904-L9911)
提供 `qwen/skills/install`、`qwen/skills/delete`、`qwen/skills/setEnabled`，
但它们不是所有 ACP Agent 都支持的标准方法。
Agent 也可以通过 [Slash commands](https://agentclientprotocol.com/protocol/v1/slash-commands)
暴露某些技能入口，BasG 已有命令列表与补全，但这不替代技能分发和生命周期管理。

因此应将后续工作拆成 **MCP 工具接入、skills 管理、委托编排** 三项，而不是把它们都归为
“MCP 支持”。本轮没有实现这些管理／编排能力，也没有接入后台子会话视图。
客户端托管终端按独立一期处理，继续声明 `terminal: false`。

## 退出登录

退出登录用于换账号、撤销 Agent 保存的登录状态，或排查认证问题。它不是卸载，
也不等于从模型服务商侧吊销所有 API key。

在 Agent 目录或设置页的已配置 ACP Agent 卡片中选择“退出登录”，确认后执行：

1. 校验 GUI 与 daemon 使用同一份本地配置，并核对 AgentSpec。
2. 建立短连接，发送 initialize；只有 Agent 声明 `agentCapabilities.auth.logout` 才发送 `logout`。
3. 不调用 `session/new`、不自动重试、不删除 Bas 配置或会话。
4. 清除旧的检测状态，显示结果。后续由用户显式检测或登录。

按钮会在操作时进行能力检查；Agent 未声明 logout 时明确报错，不猜测原生 CLI 命令，
也不自行删除其凭据文件。Agent 可能自行清理它管理的凭据。
对正在运行的会话，Agent 可能继续执行、停止或在下一次请求要求认证，确认框会提示这个影响。
环境变量中的 API key 不会被 Bas 修改。详见
[ACP 认证规范](https://agentclientprotocol.com/protocol/v1/authentication)。

## 哪些 Agent 支持音频

`promptCapabilities.audio` 只声明 **音频输入**，不能当作“音频输入＋输出”能力。
模型 API 支持语音也不证明该 Agent 的 ACP 适配器接通了它。

本轮核对的源码快照：

| Agent／适配器 | ACP 音频输入 | 核对依据 |
| --- | --- | --- |
| Gemini CLI | 已声明，并将 AudioContent 转为模型的 inlineData | [能力声明](https://github.com/google-gemini/gemini-cli/blob/ed96faedfd5571f055c44f2c9509d62321d1138d/packages/cli/src/acp/acpRpcDispatcher.ts#L91-L100)、[输入转换](https://github.com/google-gemini/gemini-cli/blob/ed96faedfd5571f055c44f2c9509d62321d1138d/packages/cli/src/acp/acpSession.ts#L962-L971) |
| Qwen Code | 已声明，并有音频输入转换；实际模型还须支持对应输入 | [能力声明](https://github.com/QwenLM/qwen-code/blob/720eff873a7fd32a0a94a1844f608a2696e26834/packages/cli/src/acp-integration/acpAgent.ts#L5515-L5520)、[输入转换](https://github.com/QwenLM/qwen-code/blob/720eff873a7fd32a0a94a1844f608a2696e26834/packages/cli/src/acp-integration/session/Session.ts#L17727-L17733) |
| claude-agent-acp | 该快照未声明音频输入 | [能力声明](https://github.com/agentclientprotocol/claude-agent-acp/blob/dbf386d8d35d2bc30f17f11f15d1a4305c9d391f/src/acp-agent.ts#L2974-L2977) |
| codex-acp | 该快照只声明图片和嵌入上下文，未声明音频输入 | [能力声明](https://github.com/agentclientprotocol/codex-acp/blob/59974c7d173e4fd6ef9498fc1e91779606c2404d/src/CodexAcpServer.ts#L664-L675) |
| pi-acp | 明确声明 false；转换路径仅保留不支持音频的文字提示 | [能力声明](https://github.com/svkozak/pi-acp/blob/04d0a15d6bb42bb65cce5c9caedcdcb2cdcb6905/src/acp/agent.ts#L259-L263)、[转换边界](https://github.com/svkozak/pi-acp/blob/04d0a15d6bb42bb65cce5c9caedcdcb2cdcb6905/src/acp/translate/prompt.ts#L69-L73) |

这不是穷尽名单，也不是对已发布版本的统一承诺；运行时以当前 Agent 的 initialize 为准。
没有据这些输入声明确认任何一个 Agent 能稳定返回生成式音频：
输出必须实际通过消息或工具内容返回 ACP AudioContent，工具写出一个文件路径不等于返回了音频块。
上述核对没有调用真实模型验证音频质量。

## BasG 本轮音频范围

- 附件选择、拖放等现有文件入口将 `audio/*` 文件构造成原生 ACP AudioContent。
  必须有 `promptCapabilities.audio`，不借用 `embeddedContext` 绕过协商。
- 返回的 AudioContent 显示播放、暂停和停止控件，不依赖 Agent 的输入能力标志。
  只有显式点击才解码和打开默认音频输出设备；历史回放不自动播放。
- 每个聊天窗口同时播放一段音频，切换会话或关闭窗口会停止播放。
  解码与设备操作在独立线程中进行，格式、设备和播放错误显示在对应控件中。
- 使用发布版 rodio／Symphonia，支持 WAV、MP3、FLAC、Ogg Vorbis、AAC／M4A 等其已启用的解码格式。
  不支持的编码明确报错；不承诺 Opus、裸 PCM 包或任意音频 MIME 都可播放。
- 沿用附件的 8 MiB 单文件、16 MiB 合计限制及 RPC 大小限制，不增加上传协议。
- 不含麦克风录音、实时语音通话、TTS 服务、波形编辑，也不会自动下载资源链接作为音频。

Linux 的 GUI 构建和运行库加入 ALSA；daemon／CLI 默认构建不依赖播放器。
回归覆盖原生附件类型与能力检查、离线 WAV 解码、坏数据错误、播放状态控制、迟到事件隔离、
真实 gRPC 退出登录与 GUI 确认；不以这些测试替代真实声卡听音或真实模型互操作验收。
