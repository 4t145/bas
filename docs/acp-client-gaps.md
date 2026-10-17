# BasG 的 ACP 客户端能力差距

## 核对范围与基线

- 核对日期：2026-10-16。
- 初始差距基线：[`a5b79a1`](https://github.com/4t145/bas/commit/a5b79a1550c599fa3f2f0fe2678c32b6d6d03004)；
  消息身份、历史互通与标准关闭／删除已随本轮实现更新，见下文已有能力。
- SDK：[`agent-client-protocol = 3.3.0`](../Cargo.toml#L49-L50)，启用实验类型和 v2 schema；
  [外部连接仍使用 ACP v1](../src/bin/bas/agent/acp.rs#L293-L295)。
- 方法：源码与协议核对，以及已落地项目的受控 ACP fixture／GUI 回归测试；
  尚未通过真实 Agent 逐项实测。协议与适配器持续演进，本文不是永久不变的支持承诺。

实际链路是 `BasG → Bas gRPC → ACP Agent`。Bas daemon 才是持有 ACP 连接、
执行客户端回调的协议客户端；BasG 提供用户界面。本文按这条链路最终向用户提供的能力判断，
区分“底层未实现”“底层已有但 GUI 未接入”和“已有展示模型但传输未接入”。

ACP 的可选能力不是所有客户端都必须实现。本文是功能差距分析，不是协议不合规清单；
稳定 v1、实验提案和纯产品交互增强也不混为一谈。

## 结论

主对话链路、消息身份和 Agent 会话互通已经接入。剩余主要差距集中在上下文与工具配置、
后台任务和新协议，而不是继续增加已有的展示控件。

- **已落地**：按 `messageId` 聚合消息，发现／导入 Agent 历史会话，标准关闭与删除。
- **明确不做**：旧式 `modes` 的 GUI 回退；面向未来，以 `configOptions` 为统一入口。
- **通用客户端能力**：标准退出登录与已有会话的副工作目录已接入；客户端 MCP 配置、skills 管理、委托编排仍待接入。
- **执行与多模态**：客户端托管终端单独分一期；本轮支持音频文件输入和返回音频的按需播放，不含录音／实时语音。
- **实验能力**：Subagent、会话分支、原生 ACP v2 和远程 ACP transport 尚未端到端接入。

## 已有能力，不应列为缺失

| 类别 | 当前能力与边界 |
| --- | --- |
| 基础对话 | 创建会话、流式回答与思考、取消回合、工具调用及局部更新。 |
| 消息身份 | 按会话、角色和 Agent 的不透明 `messageId` 聚合 chunk；不同 ID 不合并，交错 chunk 回到原消息。无 ID 时保留相邻消息聚合。 |
| Agent 历史互通 | 独立窗口按 ACP Agent ID 分组、按主工作区 `cwd` 筛选；显式发现历史，在同一 ACP 连接读完分页。导入时加载原会话并保留回放；重复导入返回同一 Bas 记录，映射与历史可持久化恢复。仅支持 resume 时明确提示没有旧消息回放。 |
| 标准关闭／删除 | 检查 Agent 能力后发送 `session/close`／`session/delete`。关闭保留 Bas 记录为已停止状态；删除 Agent 历史与删除 Bas 本地记录分别确认，互不冒充。 |
| 用户交互 | 标准工具权限审批；Elicitation 表单和 URL 交互；请求取消与等待状态。 |
| 退出登录 | GUI 确认后由 daemon 校验配置与标准 logout 能力；不创建会话、不自动重试、不自行删除配置或凭据。 |
| 文件音频 | 以 AudioContent 发送 `audio/*` 文件，仅向声明音频输入的 Agent 开放；返回 AudioContent 可播放、暂停和停止，回放不自动播放。 |
| 会话设置 | `configOptions` 的 Select／Bool 控件、更新与错误；Slash commands 目录及补全。 |
| 展示元素 | 计划、使用量与累计费用、即时通知、上下文压缩、图片、嵌入文本文件、资源链接和 v1 diff。部分展示使用实验扩展或兼容路径，不等于支持原生 v2。 |
| 文件服务 | daemon 已实现 ACP 文本文件读写回调；不是 GUI 编辑器缓冲区，也不是文件访问沙箱。 |
| Bas 会话保存与恢复 | 默认 localdb 保存 Bas 元数据与事件；对已登记会话调用 Agent 的 `session/load` 或 `session/resume` 恢复执行上下文，前提是 Agent 支持并恢复成功。 |

逐项支持范围见 [ACP 展示元素](gui.md#acp-展示元素)，传输与持久化语义见
[RPC 文档](rpc.md#回放与输入边界)；文件服务实现见
[文件服务](../src/bin/bas/agent/driver.rs#L335-L401)。
历史发现、导入与删除见 [ACP 历史驱动](../src/bin/bas/agent/acp/history.rs#L1)、
[会话注册逻辑](../src/bin/bas/session/registry/history.rs#L1)，使用方法见
[Agent 历史会话](gui.md#agent-历史会话)。

## 稳定 ACP v1 能力差距

“稳定”指 v1 已定义相关协议能力，不代表每个 Agent 都支持，也不代表客户端必须全部实现。
旧式 modes 已有配置选项替代方案，客户端托管终端则需要结合 v2 的方向决定投入。

| 功能 | 当前状态与影响 | 代码依据 |
| --- | --- | --- |
| 客户端配置 MCP servers | **未接入**：创建、加载、恢复请求未设置 MCP 配置，使用默认空列表。BasG 不能统一配置工具服务器并交给不同 Agent；Agent 自己原生配置的 MCP 不在此限制之内。 | [会话请求构造](../src/bin/bas/agent/acp.rs#L312-L340) |
| 客户端托管终端 | **未实现且未声明**：`terminal: false`，没有 `terminal/create`、`output`、`wait_for_exit`、`kill`、`release` 的执行回调。已有终端输出展示不等于由 Bas 托管进程。 | [能力声明](../src/bin/bas/agent/acp.rs#L367-L383)、[当前终端边界](gui.md#前台交互与后台任务) |
| 录音与实时语音 | **未接入**：已有完整文件的音频输入／播放，尚不提供麦克风采集、实时语音会话或 TTS 服务。 | [音频范围与 Agent 核对](acp-tools-and-audio.md#basg-本轮音频范围) |

### 消息身份与历史恢复

v1 的 `messageId` 可选；GUI 现在在 Agent 提供时使用 ID，不改变原始 chunk 的历史存储。
没有 ID 的 Agent 保留相邻同角色聚合，但不会把无 ID chunk 追加到有 ID 的消息中。
去重仍依据事件游标，不能仅按消息文本相同就合并不同提交或重放消息。

Bas 自己的列表与 Agent 的历史列表仍是两个目录，浏览不隐式导入或发送 prompt。
导入使用 Agent 的原始 ID 与工作目录，`session/load` 的回放进入 Bas 历史；
恢复已导入会话时则丢弃重复回放。仅支持 `session/resume` 的 Agent 可以恢复上下文，
但没有可回放旧消息；只有 list 能力的 Agent 不显示可用的导入按钮。
含额外工作根的外部会话暂时拒绝导入，避免静默缩减工作范围。

### 关闭、删除、归档与账号

以下操作不能静默互相替代：

- **Bas 归档**：调整本地列表可见性，不停止 Agent。
- **Bas 删除**：停止后端并遗忘 Bas 记录。
- **ACP `session/close`**：释放 Agent 端该活跃会话的资源，不是删除历史。
- **ACP `session/delete`**：从 Agent 后续的会话列表中移除记录，底层软删／硬删由 Agent 决定。
- **ACP `logout`**：结束认证状态，不等于卸载程序或删除会话。

关闭和删除均先检查 Agent 声明的能力。已导入且仍活跃的会话须先关闭，再删除 Agent 历史；
删除失败保留本地数据，不自动重试。关闭被 Agent 明确拒绝时保留运行会话；
超时则停止不确定的本地运行实例并报告失败，不宣称 Agent 已成功关闭。
关闭后的 Bas 记录是只读的已停止实例；当前没有重新打开该实例的入口。

### MCP 与多工作目录

MCP 不涵盖所有客户端扩展：skills 的分发／启用由 Agent 或其扩展决定，
委托工具涉及 Bas 的会话编排，Subagent 展示也不等于客户端委托接口。
这三项需要分别规划，具体边界见 [工具、skills 与委托](acp-tools-and-audio.md#不只有-mcp工具skills委托是不同层次)。

客户端注入 MCP 配置不要求 Bas 自己成为这些服务器的 MCP 客户端：
标准流程是通过会话生命周期请求把配置交给 Agent，由 Agent 建立连接。
HTTP／SSE 等可选 transport 需检查对应 Agent 能力，不能仅因配置能保存就判为可用。

`additionalDirectories` 是单个会话的多个工作根，不是 BasG 已有的多个 workspace 分组。
BasG 现可在空闲会话中管理副目录，能力协商后以原 ID 和 cwd 重新加载，持久化完整配置，
并在后续 load／resume 时重新发送预期根列表。含副目录的外部历史会话导入仍暂时拒绝；
工作根提示本身也不等于对 Agent 进程实施了操作系统级文件隔离。

## 实验、草案与新版本能力

这部分不计入稳定 v1 的完整性要求。启用 SDK 实验类型不等于协商了能力或打通了传输。

| 功能 | 当前差距 | 依据 |
| --- | --- | --- |
| Subagent 子会话 | 已声明 v1 subagents，接入父子关联、嵌套路由、事件持久化与工作状态浮层。仍缺独立子会话全文界面、单独取消和真实 Agent 互操作验收；不会把普通工具调用冒充子 Agent。 | [官方提案](https://agentclientprotocol.com/rfds/subagents)、[当前后台任务边界](gui.md#前台交互与后台任务) |
| 会话分支 `session/fork` | 没有将现有上下文复制为新会话的 RPC 或 GUI 入口。提案用于不污染原对话地尝试其他路线；不能把它解释成已经提供“从任意历史消息位置分叉”。 | [官方提案](https://agentclientprotocol.com/rfds/session-fork)、[当前会话 RPC](../proto/bas/v1/bas.proto#L26-L65) |
| 原生 ACP v2 | 握手固定 v1；部分 v2 状态和终端展示处理已实现，但 prompt 插入确认／前台状态生命周期、整条消息 upsert、工具内容 chunk、结构化多文件 diff 和原生终端事件尚未端到端接入。 | [官方迁移说明](https://agentclientprotocol.com/protocol/v2/migration)、[握手](../src/bin/bas/agent/acp.rs#L293-L295)、[已有展示处理](../src/bin/basg/conversation/protocol.rs#L57-L130) |
| 远程 ACP transport | 当前通过本地子进程 stdio 连接，没有 ACP-over-HTTP／WebSocket。BasG 的 `--server` 是 Bas 的 gRPC 地址，不是远程 ACP Agent 地址。 | [官方提案](https://agentclientprotocol.com/rfds/streamable-http-websocket-transport)、[子进程传输构造](../src/bin/bas/agent/acp/convert.rs#L6-L16) |

后台任务接入还需要把前台状态与任务状态分开：主会话空闲或 prompt 完成，
不代表所有子任务都结束；断线后的最后一次“运行中”也不能继续当作实时状态。
当前子会话接收链路已在 daemon、RPC、持久化与 GUI 中保持这些区别；完整任务控制仍待接入。

## 产品增强与适配器限制

- **Diff 审阅**：已有 v1 diff 展示，但没有专门的 diff 审阅／接受／拒绝界面，
  也没有 v2 的多文件新增、删除、移动等结构化变更清单。
  [当前展示边界](gui.md#acp-展示元素)不应被描述为“没有工具权限审批”；
  是否能在修改前阻止执行，还取决于 Agent 主动请求权限。
- **原生 Agent 的交互不一定已转成 ACP**：BasG 已支持标准 Elicitation，
  但适配器不发请求时，客户端不会从 Markdown 中猜出表单。
  已记录的 Pi／Codex 适配边界见 [前台交互与后台任务](gui.md#前台交互与后台任务)；
  它们不应被重复列为 BasG 缺少标准表单能力。

## 建议实施顺序

以下区分已落地项目和后续路线，不表示未完成项目已排期。

| 顺序 | 工作 | 建议验收重点 |
| --- | --- | --- |
| 已落地：消息身份 | 消费 `messageId` | 不同 ID 的同角色消息不合并，同一 ID 的交错 chunk 正确归属；无 ID 时保留聚合；直播与回放一致。旧式 modes GUI 回退不在计划内。 |
| 已落地：会话互通 | Agent 历史发现／导入；标准关闭／删除 | 能力检查、同连接分页、Agent ID 到 Bas ID 的映射、原会话恢复、重启与重复导入、超时及原生错误、GUI 草稿与迟到响应隔离。 |
| 3：工具与上下文 | MCP、skills、委托、多工作目录 | 分别明确标准能力、Agent 扩展和 Bas 编排责任；配置穿过完整链路并在恢复时保留，能力不足时明确提示。标准退出登录已落地。 |
| 4：并行工作与新协议 | Subagent 与任务视图；v1／v2 并存 | 子会话事件与交互归属正确，单独取消遵循能力声明；重连不把历史状态当成当前状态；v2 接收确认与前台完成分离，v1 不回归。 |
| 按场景决定 | 实时语音、diff 审阅、远程连接 | 文件音频已落地；进一步能力依据真实 Agent 与使用场景推进。 |
| 单独一期 | 客户端托管终端 | 本轮不实现、不声明；单独设计进程所有权、生命周期及与后续协议的关系。 |

不建议仅为补全 v1 优先实现客户端托管终端。
[v2 移除了客户端文件与终端执行接口](https://agentclientprotocol.com/protocol/v2/migration)，
建议通过客户端提供的 MCP 工具扩展执行面；Agent 自己执行后的终端展示仍需要保留。
这不影响维护已有 v1 文件读写兼容能力。

## 后续验证要求

本轮已通过 `cargo test --all-targets --locked` 和 GUI shell 中的
`cargo test --features gui-tests --bin basg --locked`。
新增的受控 ACP fixture 覆盖分页、导入回放、原会话续聊、重启恢复、关闭／删除边界、
能力检查、错误与超时；GUI 测试覆盖草稿保留与迟到响应隔离。
尚未运行真实 Agent 互操作流程或 QEMU 系统测试，仍需用确实声明对应能力的 Agent 验证完整路径。
“schema 中存在类型”“RPC 能传 JSON”和“Agent 原生 CLI 有此功能”都不是端到端支持的证据。

维护本清单时，应同时更新核对日期与代码基线；已落地项移入已有能力，
仍受适配器限制的项目单独注明，避免旧缺口继续作为待办流传。

## 协议参考

- [ACP v1 会话创建、加载、恢复、关闭、MCP 与额外工作根](https://agentclientprotocol.com/protocol/v1/session-setup)
- [ACP v1 会话列表](https://agentclientprotocol.com/protocol/v1/session-list)
- [ACP v1 会话删除](https://agentclientprotocol.com/protocol/v1/session-delete)
- [ACP 消息 ID](https://agentclientprotocol.com/rfds/message-id)
- [ACP v1 会话模式](https://agentclientprotocol.com/protocol/v1/session-modes)
- [ACP v1 认证与退出登录](https://agentclientprotocol.com/protocol/v1/authentication)
- [ACP v1 终端](https://agentclientprotocol.com/protocol/v1/terminals)
- [ACP v1 内容类型](https://agentclientprotocol.com/protocol/v1/content)
- [ACP v2 迁移说明](https://agentclientprotocol.com/protocol/v2/migration)
