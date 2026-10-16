# BasG 的 ACP 客户端能力差距

## 核对范围与基线

- 核对日期：2026-10-16。
- 代码基线：[`a5b79a1`](https://github.com/4t145/bas/commit/a5b79a1550c599fa3f2f0fe2678c32b6d6d03004)。
- SDK：[`agent-client-protocol = 3.3.0`](../Cargo.toml#L49-L50)，启用实验类型和 v2 schema；
  [外部连接仍使用 ACP v1](../src/bin/bas/agent/acp.rs#L293-L295)。
- 方法：静态核对源码、仓库文档与 ACP 官方文档，没有通过真实 Agent 逐项实测。
  协议与适配器持续演进，本文描述的是该基线的能力，不是永久不变的支持承诺。

实际链路是 `BasG → Bas gRPC → ACP Agent`。Bas daemon 才是持有 ACP 连接、
执行客户端回调的协议客户端；BasG 提供用户界面。本文按这条链路最终向用户提供的能力判断，
区分“底层未实现”“底层已有但 GUI 未接入”和“已有展示模型但传输未接入”。

ACP 的可选能力不是所有客户端都必须实现。本文是功能差距分析，不是协议不合规清单；
稳定 v1、实验提案和纯产品交互增强也不混为一谈。

## 结论

主对话链路已经可用。主要差距集中在消息身份、Agent 会话互通、上下文与工具配置、
后台任务，而不是继续增加已有的展示控件。

- **兼容性正确性**：尚未使用 `messageId` 区分消息；缺少旧式 `modes` 的 GUI 回退入口。
- **通用客户端能力**：缺少 Agent 历史会话发现／导入、标准关闭／删除／退出登录、
  客户端 MCP 配置和单会话多工作目录。
- **执行与多模态**：未实现客户端托管终端；音频没有完整输入／播放体验。
- **实验能力**：Subagent、会话分支、原生 ACP v2 和远程 ACP transport 尚未端到端接入。

## 已有能力，不应列为缺失

| 类别 | 当前能力与边界 |
| --- | --- |
| 基础对话 | 创建会话、流式回答与思考、取消回合、工具调用及局部更新。 |
| 用户交互 | 标准工具权限审批；Elicitation 表单和 URL 交互；请求取消与等待状态。 |
| 会话设置 | `configOptions` 的 Select／Bool 控件、更新与错误；Slash commands 目录及补全。 |
| 展示元素 | 计划、使用量与累计费用、即时通知、上下文压缩、图片、嵌入文本文件、资源链接和 v1 diff。部分展示使用实验扩展或兼容路径，不等于支持原生 v2。 |
| 文件服务 | daemon 已实现 ACP 文本文件读写回调；不是 GUI 编辑器缓冲区，也不是文件访问沙箱。 |
| Bas 会话保存与恢复 | 默认 localdb 保存 Bas 元数据与事件；对已登记会话调用 Agent 的 `session/load` 或 `session/resume` 恢复执行上下文，前提是 Agent 支持并恢复成功。 |

逐项支持范围见 [ACP 展示元素](gui.md#acp-展示元素)，传输与持久化语义见
[RPC 文档](rpc.md#回放与输入边界)；文件服务实现见
[`LocalHost`](../src/bin/bas/agent/driver.rs#L324-L390)，已有恢复分支见
[`Startup::Restore`](../src/bin/bas/agent/acp.rs#L318-L343)。

## 稳定 ACP v1 能力差距

“稳定”指 v1 已定义相关协议能力，不代表每个 Agent 都支持，也不代表客户端必须全部实现。
旧式 modes 已有配置选项替代方案，客户端托管终端则需要结合 v2 的方向决定投入。

| 功能 | 当前状态与影响 | 代码依据 |
| --- | --- | --- |
| 按 `messageId` 区分消息 | **部分支持**：消息 chunk 可以进入历史，但 GUI 只取 `content`，未用 Agent 的消息 ID 聚合。相邻、同角色、仍在流式状态的不同消息可能合并；被工具等事件分隔的同一消息也没有按 ID 归并。 | [消息接收](../src/bin/basg/conversation.rs#L145-L159)、[chunk 合并](../src/bin/basg/conversation.rs#L247-L271) |
| Agent 历史会话发现／导入 | **未接入**：列表来自 Bas 自己的注册表，没有调用 ACP `session/list`，也没有把外部 Agent session ID 导入 Bas 的入口。无法直接浏览并接续原生 CLI 或其他客户端创建的会话。 | [Bas 列表](../src/bin/bas/service/server.rs#L467-L482)、[当前创建／恢复路径](../src/bin/bas/agent/acp.rs#L312-L344) |
| Agent 端标准关闭／删除 | **未接入**：没有发送 ACP `session/close` 或 `session/delete`。Bas 删除操作是停止后端并遗忘本地记录，不保证从 Agent 的历史列表删除。 | [Bas 删除入口](../src/bin/bas/service/server.rs#L496-L506)、[后端 shutdown](../src/bin/bas/agent/acp.rs#L589-L597) |
| 客户端配置 MCP servers | **未接入**：创建、加载、恢复请求未设置 MCP 配置，使用默认空列表。BasG 不能统一配置工具服务器并交给不同 Agent；Agent 自己原生配置的 MCP 不在此限制之内。 | [会话请求构造](../src/bin/bas/agent/acp.rs#L312-L340) |
| 单会话多工作目录 | **未接入**：只有一个 `cwd`，没有 `additionalDirectories` 的配置、传输和恢复。不能通过标准 ACP 为同一会话指定主项目、共享库等多个工作根。 | [创建参数](../src/bin/bas/service/server.rs#L437-L459)、[ACP 请求](../src/bin/bas/agent/acp.rs#L312-L340) |
| 旧式 `modes` GUI 入口 | **底层已有，GUI 未接**：daemon／RPC 已支持 `session/set_mode`，BasG 只根据 `configOptions` 生成控件。只返回 `modes.availableModes` 的 Agent 缺少模式切换入口。 | [已有 RPC](../proto/bas/v1/bas.proto#L50-L53)、[GUI 配置入口](../src/bin/basg/workspace/agent.rs#L149-L160) |
| 标准退出登录 `logout` | **未接入**：已有登录与交互认证，但没有标准退出登录的 RPC／GUI 流程。移除托管 Agent 保留凭据，不等于退出账号。 | [管理／认证接口](../proto/bas/v1/bas.proto#L20-L25) |
| 客户端托管终端 | **未实现且未声明**：`terminal: false`，没有 `terminal/create`、`output`、`wait_for_exit`、`kill`、`release` 的执行回调。已有终端输出展示不等于由 Bas 托管进程。 | [能力声明](../src/bin/bas/agent/acp.rs#L367-L383)、[当前终端边界](gui.md#前台交互与后台任务) |
| 音频输入／输出 | **部分支持**：RPC 能承载 ACP Audio，但 GUI 收到音频只显示 MIME 类型文字，没有播放器；选择音频文件也走普通资源附件路径，没有专门构造 `AudioContent` 或录音入口。 | [音频展示](../src/bin/basg/workspace/chat/content.rs#L705-L707)、[附件构造](../src/bin/basg/attachment.rs#L75-L108) |

### 消息身份与历史恢复

v1 的 `messageId` 可选，应在 Agent 提供时使用；没有 ID 的旧 Agent 仍需保留兼容路径。
当前缺口是 GUI 未消费 ID，不能据此认定 RPC 或存储丢弃了整个原始 chunk。
也不能仅按消息文本相同就合并不同提交或重放消息。

Bas 自己的列表与 Agent 的历史列表是两个目录。现有恢复代码依赖 Bas 已保存的后端 session ID；
支持 `session/load`／`session/resume` 不等于已有外部会话发现和导入。
新增发现能力时，应区分“能列出”“能读取历史”和“能恢复执行上下文”，不能只列出标题就宣称可继续对话。

### 关闭、删除、归档与账号

以下操作不能静默互相替代：

- **Bas 归档**：调整本地列表可见性，不停止 Agent。
- **Bas 删除**：停止后端并遗忘 Bas 记录。
- **ACP `session/close`**：释放 Agent 端该活跃会话的资源，不是删除历史。
- **ACP `session/delete`**：从 Agent 后续的会话列表中移除记录，底层软删／硬删由 Agent 决定。
- **ACP `logout`**：结束认证状态，不等于卸载程序或删除会话。

接入这些操作时，应先检查 Agent 广告的能力，并在 GUI 中明确操作影响范围；
不能因为它们与现有 Bas RPC 名字相近就直接复用语义。

### MCP 与多工作目录

客户端注入 MCP 配置不要求 Bas 自己成为这些服务器的 MCP 客户端：
标准流程是通过会话生命周期请求把配置交给 Agent，由 Agent 建立连接。
HTTP／SSE 等可选 transport 需检查对应 Agent 能力，不能仅因配置能保存就判为可用。

`additionalDirectories` 是单个会话的多个工作根，不是 BasG 已有的多个 workspace 分组。
新增后应持久化完整配置，并在 load／resume 时重新发送预期根列表；
工作根提示本身也不等于对 Agent 进程实施了操作系统级文件隔离。

## 实验、草案与新版本能力

这部分不计入稳定 v1 的完整性要求。启用 SDK 实验类型不等于协商了能力或打通了传输。

| 功能 | 当前差距 | 依据 |
| --- | --- | --- |
| Subagent 子会话 | 未声明 v1 subagent 能力，没有父子会话路由、子会话独立消息／计划／状态与单独取消。当前只能展示适配器折叠进普通工具调用的进度。 | [官方提案](https://agentclientprotocol.com/rfds/subagents)、[当前后台任务边界](gui.md#前台交互与后台任务)、[当前单会话通知校验](../src/bin/bas/agent/acp.rs#L629-L642) |
| 会话分支 `session/fork` | 没有将现有上下文复制为新会话的 RPC 或 GUI 入口。提案用于不污染原对话地尝试其他路线；不能把它解释成已经提供“从任意历史消息位置分叉”。 | [官方提案](https://agentclientprotocol.com/rfds/session-fork)、[当前会话 RPC](../proto/bas/v1/bas.proto#L26-L65) |
| 原生 ACP v2 | 握手固定 v1；部分 v2 状态和终端展示处理已实现，但 prompt 插入确认／前台状态生命周期、整条消息 upsert、工具内容 chunk、结构化多文件 diff 和原生终端事件尚未端到端接入。 | [官方迁移说明](https://agentclientprotocol.com/protocol/v2/migration)、[握手](../src/bin/bas/agent/acp.rs#L293-L295)、[已有展示处理](../src/bin/basg/conversation/protocol.rs#L57-L130) |
| 远程 ACP transport | 当前通过本地子进程 stdio 连接，没有 ACP-over-HTTP／WebSocket。BasG 的 `--server` 是 Bas 的 gRPC 地址，不是远程 ACP Agent 地址。 | [官方提案](https://agentclientprotocol.com/rfds/streamable-http-websocket-transport)、[子进程传输构造](../src/bin/bas/agent/acp/convert.rs#L6-L16) |

后台任务接入还需要把前台状态与任务状态分开：主会话空闲或 prompt 完成，
不代表所有子任务都结束；断线后的最后一次“运行中”也不能继续当作实时状态。
这些语义需要落到 daemon、RPC、持久化与 GUI，而不只是增加一个任务面板。

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

以下是路线建议，不表示这些功能已排期或已承诺实现。

| 顺序 | 工作 | 建议验收重点 |
| --- | --- | --- |
| 1：兼容性正确性 | 消费 `messageId`；旧式 `modes` GUI 回退 | 不同 ID 的同角色消息不合并，同一 ID 的 chunk 正确归属；无 ID 的旧 Agent 保持兼容。优先使用 `configOptions`，缺省时才显示旧式 modes，不产生重复控件。 |
| 2：会话互通 | Agent 历史发现／导入；标准关闭／删除 | 能力检查、分页、Agent session ID 到 Bas ID 的映射；明确本地记录与 Agent 历史的区别，导入后恢复上下文而非偷偷创建新对话。 |
| 3：配置与账号 | MCP、多工作目录、退出登录 | 配置能穿过 GUI→RPC→daemon→ACP，并在恢复时保留；能力不足时明确提示。凭据不进入诊断日志；退出登录不静默删除会话。 |
| 4：并行工作与新协议 | Subagent 与任务视图；v1／v2 并存 | 子会话事件与交互归属正确，单独取消遵循能力声明；重连不把历史状态当成当前状态；v2 接收确认与前台完成分离，v1 不回归。 |
| 按场景决定 | 音频、diff 审阅、远程连接、客户端托管终端 | 根据真实 Agent 与使用场景验证，不为能力表打勾而实现未使用的执行面。 |

不建议仅为补全 v1 优先实现客户端托管终端。
[v2 移除了客户端文件与终端执行接口](https://agentclientprotocol.com/protocol/v2/migration)，
建议通过客户端提供的 MCP 工具扩展执行面；Agent 自己执行后的终端展示仍需要保留。
这不影响维护已有 v1 文件读写兼容能力。

## 后续验证要求

本文没有运行构建、测试或真实 Agent 互操作流程，不能替代实现验收。
实施时建议先用受控 ACP fixture 覆盖能力协商、消息身份、请求错误、取消和重连，
再用确实广告对应能力的真实 Agent 验证 GUI 完整路径。
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
