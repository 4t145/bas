# RPC 与 ACP 类型

Bas 的会话交互复用 `agent-client-protocol` 3.3.0 提供的 schema，
与 2026-10-10 核对的 [Zed 基线](upstream-ui.md#acp-展示协议基线2026-10-10)一致。
外部 Agent 连接仍协商 ACP v1，RPC 与历史使用 `bas::acp`；
`bas::acp_v2` 用于新版计划、用量、通知、压缩等展示模型。
SDK 启用 `process`、`schemars`、`unstable`、`unstable_protocol_v2`；
编译实验类型不改变握手版本，也不自动声明未实现的客户端能力。

gRPC 保留会话管理与后台运行所需的信封。`AcpPayload.json` 是 UTF-8 JSON bytes，
直接由 ACP SDK 的 Serde 实现编解码，不在 protobuf 中再定义一套 ContentBlock、
ToolCall、ConfigOption 或 StopReason，也不经过会把 JSON 整数转为 double 的 Struct。
SDK 已知字段、annotations 和 `_meta` 保留原有语义；对未知字段和无效可选项的处理遵循 SDK。

ACP `fs/read_text_file` 读取服务端所在机器上的 UTF-8 文本文件，不是图片读取接口。
非 UTF-8 数据会返回 `InvalidParams`，错误消息明确说明文本限制；文件系统错误保留具体原因。
图片通过 ACP `ImageContent` 内容块传输，是否支持输入图片由智能体声明的 prompt capability 决定。
TCP、mTLS 和本地 IPC 不改变这两条路径的含义。

| gRPC 接口或字段 | ACP 类型 / Bas 职责 |
| --- | --- |
| ListAgents、CreateSession、ListSessions、GetSession、DeleteSession | 后端选择、进程与会话生命周期 |
| ListAgentSessions | 在单条 ACP 连接上调用 `session/list` 并读完分页；返回原生 `ListSessionsResponse`、初始化能力和已导入 ID 映射，不创建会话 |
| ImportAgentSession | 接收 Agent 原生 `SessionInfo`，加载／恢复指定会话，保存 Bas UUID 与 Agent ID 映射；不调用 `session/new` |
| CloseSession | 停止运行实例；支持时先调用 `session/close`，否则释放本地实例，保留 Bas 历史 |
| RestartSession | 显式重新启动已停止且未归档的会话，保留本地 ID 和历史，不发送 prompt |
| SetSessionDirectories | 替换已有空闲会话的完整副目录列表；原 ID、原 cwd 加载／恢复，不创建替代会话 |
| Session.additional_directories、WorkspaceChanged | 持久化的副目录与重新加载后配置；按事件游标传播，不改变主目录分组 |
| DeleteAgentSession | 能力协商后调用 `session/delete`，只删除 Agent 历史；已导入活跃会话须先关闭 |
| ListAgentCatalog | `bas::management::Catalog` JSON，包含 Registry 和已配置后端 |
| InstallAgent | 可取消的安装进度流，最终返回已保存的 `AgentSpec` |
| RemoveAgent | 移除托管注册，保留原生凭据与版本目录 |
| AuthenticateAgent | daemon 持有 ACP 连接执行标准 agent 认证，再检查 session/new |
| LogoutAgent | 校验 AgentSpec 后握手，仅在声明 `agentCapabilities.auth.logout` 时调用标准 `logout`；不创建或删除会话 |
| Session.initialization | `InitializeResponse`，包含 Agent 能力、实现信息与认证方法 |
| Session.acp | `NewSessionResponse`，包含当前 modes、configOptions 和初始扩展元数据 |
| Session.available_commands | 最新 `AvailableCommandsUpdate`，不受事件历史裁剪影响 |
| Session.subagents | 当前已公告子会话的父子关系、名称、描述、ACP 状态与能力快照，包含嵌套子会话 |
| Event.subagent_update | 原 ACP 子会话 `SessionNotification`；外层 session 仍为 Bas 主会话 UUID，内层保留子会话原 ID |
| Session.revision、active_turn | 快照包含的最后事件游标，以及当前正在执行的回合 |
| Session.last_reply | 最近一条 Agent 回复块的游标；用于未读判断，改名/归档等元数据操作不会推进 |
| Session.agent_updated_at | Agent 的 `session_info_update.updatedAt`，与 daemon 的事件更新时间分别保存 |
| Session.archived、SetArchived、ArchiveChanged | 归档先停止，恢复先启动；保留原会话身份和历史 |
| Prompt 请求 | `PromptRequest`，完整 prompt 内容与 `_meta` |
| Prompt 返回 | `TurnAccepted`，包含 turn ID 和对应 `TurnStarted` 的 cursor |
| TurnFinished.response | 完整 `PromptResponse`，包括 stopReason 和 `_meta` |
| TurnFinished.error、Failed | 单轮请求失败、不可继续的后端故障，分别处理 |
| Cancel 请求 | `CancelNotification` |
| SetConfigOption | `SetSessionConfigOptionRequest` / `SetSessionConfigOptionResponse` |
| SetMode | `SetSessionModeRequest` / `SetSessionModeResponse` |
| Event.acp_update | v1 `SessionUpdate`，包含内容块、工具局部更新、计划、命令、选项、用量、会话信息、通知和压缩更新 |
| Attach、LoadHistory | 带游标的有界历史与实时订阅 |
| SetForeground | 双向流报告前台会话，逐条确认；流断开自动释放状态 |
| SetTitle | Bas 本地会话标题 |

`SetSessionDirectories` 接收绝对目录列表，在 daemon 主机验证、规范化并去重；
检查 Agent 的 additionalDirectories 和 load／resume 能力。修改期间拒绝新的 prompt，
并与关闭、恢复、删除互斥。成功响应的有序回调等待目录与配置原子持久化后才结束，
即使 RPC 调用方断开也不留下半次修改。Agent 拒绝时保留旧目录；超时等未知结果使运行实例失败，
不自动重试。已保存目录在重启／恢复路径中重发。此接口不改变 Agent 的操作系统沙箱策略。

客户端始终使用 Bas 的公开 session UUID。后端驱动负责在发给 Agent 的请求中
换成 Agent 的 sessionId，返回的 `Session.acp.sessionId` 也使用公开 UUID。
权限与文件回调在 daemon 内使用 ACP 请求及权限结果类型；文件操作和固定权限策略
仍由 daemon 执行。客户端托管的终端执行能力仍未声明；初始化时通过
`clientCapabilities._meta.terminal_output = true` 接收
[Zed 的展示用终端扩展](https://github.com/zed-industries/zed/blob/main/crates/agent_servers/src/acp.rs)。
工具更新的 `_meta.terminal_info`、`terminal_output`、`terminal_exit` 原样进入历史，
GUI 按终端 ID 累积输出并渲染，`rawOutput` 作为没有可展示内容时的兜底。
终端目录与退出信息也保留，空输出的退出事件仍可展示。
初始化还声明 `plan`、`session.compaction` 和 `session.notices`；
普通会话连接还声明 `subagents: {}`。子会话必须先由已知父会话公告，不能变更所有权或形成循环；
子事件不随主回合结束而丢弃，也不进入主回答。子 Notice 仍为实时事件而非历史。
恢复后旧活动状态先失效，只有 Agent 重新报告的状态才用于工作计数；
load／resume 重放恢复子会话快照，但不会把已有子消息再次追加进历史。
这些 v1 实验事件经 RPC 到达 GUI 后转为 v2 展示类型。
这些能力在初始化时协商，更新 daemon 后应新建 session。这不是一个代理全部 ACP 方法的通用网关。

Rust 客户端示例：

```rust,no_run
use bas::{acp, service::Client};

# async fn example(mut client: Client, session: String) -> Result<(), tonic::Status> {
let request = acp::PromptRequest::new(session.clone(), vec![
    acp::TextContent::new("分析这些内容").into(),
    acp::ImageContent::new("BASE64_IMAGE_DATA", "image/png").into(),
]);
let accepted = client.prompt_acp(&request).await?;
// 使用 Attach 等待 accepted.turn 对应的 TurnFinished。
let options = client.set_config_option_acp(
    &acp::SetSessionConfigOptionRequest::new(session, "model", "agent-advertised-value"),
).await?;
# Ok(())
# }
```

发送前根据 `initialization.agentCapabilities.promptCapabilities` 判断 Agent 是否支持
图片、音频和嵌入上下文。配置修改返回完整选项列表，daemon 同时发布
`ConfigOptionUpdate` 并更新会话快照，所有订阅者都能看到变化。
配置和模式请求的 ACP 错误保留在 gRPC status details 中，Rust 客户端可用
`bas::service::acp_error` 解码原始 code、message、data。
设置响应和通知由后端有序提交；调用者取消等待不会撤销已经执行的设置或丢失其状态更新。

### Agent 历史与导入

`ListAgentSessions` 可按绝对 `cwd` 过滤；所有 ACP 页在同一连接中读取，
不把不透明游标保存到 Bas。重复游标返回错误，超过 10,000 条时要求按 workspace 过滤，
整个发现过程受 daemon 启动超时限制。GUI 独立历史窗口按 ACP Agent ID 分组，
将所选主工作区的完整 `cwd` 传入此 RPC，并在结果上继续本地搜索和校验主目录。
全部 Agent 模式最多同时执行两个发现请求；切换筛选时隔离旧响应，不无限启动 Agent 进程。

`ImportAgentSession` 使用 Agent 原始 session ID 与工作目录，并为 Bas 分配公开 UUID。
同一 Agent／原始 ID 的导入串行去重，已有记录不会重复加载。
首次 `session/load` 回放保存到 Bas，响应前排空有界更新队列；后续恢复 Bas 已有会话时仍丢弃重复回放。
只有 resume 能力时恢复上下文但不伪造历史；load／resume 都不支持则失败，不创建替代会话。
当前拒绝包含额外工作根的 `SessionInfo`，以免无声改变执行上下文。

`CloseSession` 保留本地记录并将运行实例置为 Stopped。
`RestartSession` 仅接受健康、已停止且未归档的记录；ACP 使用原后端 ID，且必须支持 load/resume。
已归档记录须通过 `SetArchived(false)` 恢复。启动与回放在提交新运行实例前隔离，
回放不会再次追加到本地历史。加载被拒绝、能力不支持或提交前取消时仍保留停止记录。
关闭、删除、启动及 daemon 退出通过同一会话恢复锁互斥；持久化提交一旦开始，
与其他会话操作一样独立于 RPC 调用者完成，因此超时后应查询实际状态，不自动重试。
Agent 拒绝关闭时保留原实例，传输超时则释放不确定的本地实例并返回失败。
进程清理也会在支持时尽力发送一次标准 close，但不把清理失败视为远端成功。
`DeleteAgentSession` 与原有本地 `DeleteSession` 分开，前者不删除本地历史，
后者不调用 Agent 历史删除。关闭／删除均保留原生 ACP 错误，不自动重试修改请求。

### 连接检测与待认证状态

`CheckAgent` 只接受与 daemon 当前配置完全一致的 `expected_spec_json`；
`connect=false` 仅检查程序，`connect=true` 执行有启动超时限制的 initialize/newSession。
返回 `AuthenticationRequired` 表示 ACP 初始化成功、但 Agent 需要认证才能创建会话。
此时 `initialization` 保留完整 `InitializeResponse`，包括 `authMethods`；
`authentication_error` 保留原始 ACP 错误，错误中显式给出的 `data.authMethods` 优先，
缺省时才使用初始化广告的方法。
客户端不能将“已找到程序”“ACP 已连接”和“会话可用”混为同一状态。

正常 `CreateSession` 的 ACP `AuthRequired` 映射为 gRPC `UNAUTHENTICATED`，
原始 ACP 错误保存在 status details，保留扩展字段；初始化广告的方式与错误来源分别读取。
GUI 遇到认证需求时重新执行 `CheckAgent`，取得完整初始化响应和当次错误。
失败的启动不注册空会话、不发 prompt，所拥有的后端进程照常回收。
终端认证能力 `auth.terminal` 与代理工具用的 `terminal` 执行能力不同；
声明前者不会授权 Agent 在连接检测时执行主机终端命令。

### 后端目录、安装与认证

`ListAgentCatalog` 包含未安装、缺失程序和不支持的分发条目，不改变 `ListAgents`
只列出可启动配置的含义。`catalog_json` 使用公开 `bas::management::Catalog` 的 Serde 类型。
`refresh=true` 更新官方目录；失败时保留上次有效缓存并返回 warning。
本地与远程目录均携带启动配置；所有已接入客户端共享此 daemon 的执行环境。

`GetAgentConfiguration` 返回 Agent 注册数组、当前 daemon profile 标识和配置段 revision。
`SaveAgentConfiguration` 必须携带读取时的 profile 和 revision，仅写入 `daemon.agents`。
profile 不符或启动时使用固定目录时返回 `FAILED_PRECONDITION`；
配置已被其他客户端修改时返回 `ABORTED`，不会覆盖新值。
服务端解析托管注册、校验完整目录并保存，再更新运行中的目录。
服务端路径不是客户端本机路径；接口不支持任意文件或前端配置上传。

`InstallAgent` 从 Registry ID 解析固定版本和平台分发，在 daemon 主机安装并保存启动配置。
`AgentOperation` 的 phase/message 表示进度；`complete=true` 且 spec_json 非空表示注册已完成。
客户端放弃进度流会取消未完成的下载或安装。最终原子注册阶段可能已完成，
取消后应重新读取目录确定状态。daemon 关闭也会结束安装。
安装与移除保留现有会话；更新产生独立启动路径。

安装、移除、检查和认证支持所有已启用传输；显式目录覆盖时配置编辑和托管安装不可用。
`AuthenticateAgent` 同样要求配置快照一致，重新握手核对当前广告的 agent 方法 ID，
在同一连接调用 authenticate 和 session/new。terminal 方法应由客户端在交互终端执行，
然后重新检查 ACP 连接，不能传给这个 RPC。

`AuthenticateAgentInteractive` 是双向认证流。客户端首先发送 `start`，包含配置快照、
方法 ID 与 `expected_method_json`；推荐传入共享 `authentication::Method` 的完整 JSON，
包括原始描述符、来源和执行计划。daemon 重新发现并核对方法后才执行。
后续事件包含交互请求、URL 完成、取消、终端执行计划和最终检测结果。
客户端使用 daemon 生成的请求 ID 回答 Form / URL，不能把 Agent 的 URL ID 当作回答 ID。

两种认证 RPC 的失败提示保留执行阶段、ACP 错误码及 Agent 返回的文字原因。
额外诊断只接受 `data` 字符串或单层的 `data.message` / `data.details` 字符串，
不把完整原始数据、堆栈或凭据对象展开为提示。Agent 方法的浏览器回调由 Agent 自己
处理，daemon 等待 `authenticate` 响应；网页完成授权之后，Agent 仍可能在账号、
项目初始化或配置保存阶段返回错误。认证请求成功后才执行会话可用性检测。

初始化响应、错误和方法均保留原始 JSON。标准 Terminal 参数追加到配置命令；旧
`_meta["terminal-auth"]` 按完整命令处理，经过验证的 Qwen 0.25.0 描述符只替换应用参数。
daemon 从托管安装记录验证启动边界，保留 Node 入口。无法确定边界的自定义包装命令
返回明确错误，提示按 Agent 说明手动配置。终端事件中的程序、参数、环境按字面执行，
客户端报告真实退出码后由 daemon 再次检测；打开终端本身不表示认证成功。

Form 支持平面字符串、数值、整数、布尔、枚举与多选，校验必填、默认值与约束。
URL 由用户确认后打开，接受请求后仍等待该连接的 `elicitation/complete`。
认证过程中关闭输入流、窗口或取消会释放 ACP 连接和待处理交互。
交互内容不进入会话历史；凭据的输入与保存由 Agent 的终端或网站管理。

`LogoutAgent { agent, expected_spec_json }` 仅接受 loopback 调用，沿用本地配置核对，
并在同一条短连接里先 initialize、再检查标准 logout 能力。不支持时不发送该方法。
请求受 daemon 启动超时约束，不自动重试，也不通过 `session/new` 验证退出结果。
原生错误经 status details 保留；GUI 清除旧检测状态，后续由用户显式检测或登录。
Bas 不改变本地会话、Agent 配置和环境变量；Agent 对已有运行会话及凭据的处理由其实现决定。

### 快照与回合确认

`Session.revision` 表示快照已包含的最后一条事件。客户端重放聊天内容时维护自己的
内容游标，但只有更高 revision 的事件才可以更新用于调度的会话元数据。
目录刷新不能被迟到的旧事件覆盖；`updated_at` 是显示用墙上时钟，不是版本或发送确认。

`TurnAccepted.cursor` 与 `turn` 一起标识被接受的请求。客户端必须确认对应回合或足够新的
会话状态，才能派发下一条排队消息；无关的标题变更不能解除发送屏障。
旧 daemon 缺少这些新增字段时，数值为零，不能将零版本当成可靠的进度确认。
建议 daemon、CLI、GUI 一起更新。

客户端按会话保存已读回复游标，用 `last_reply` 判断后台 thread 的未读状态；
不能将任意 `revision` 或 `updated_at` 变化当成新回复。旧服务缺少 `last_reply` 时，
客户端只使用实际收到的回复事件，不从目录时间戳猜测未读。

后端返回有效的 ACP 请求错误时，只结束当前回合，允许修正输入后继续原会话。
连接断开、协议故障和超过取消宽限期等不可继续的情况仍会使整个会话失败并回收后端。

### 交互工具审批

`WatchPermissions { session }` 订阅当前仍待处理的 ACP 权限请求快照。每条请求有
daemon 生成的唯一 `request_id` 和完整 `RequestPermissionRequest`；新订阅者收到当前
快照，已回答的请求不会通过历史重放。GUI 在加载会话历史前建立审批订阅。

`RespondPermission { session, request_id, response }` 返回原生 ACP
`RequestPermissionResponse`。服务端校验会话、待审批 ID 和 Agent 实际提供的 option ID；
未知选项返回 `InvalidArgument`，已失效、已回答或其他会话的请求返回 `FailedPrecondition`。
有效响应只消费请求一次，随后发布移除该请求的快照；客户端不自动重试修改请求。

回合取消、后端回调结束或会话删除会清理挂起请求。最后一个审批订阅断开时取消尚未
回答的请求；进入过交互审批的会话在无订阅期间取消后续请求，不能回退为自动允许。
从未建立交互审批的会话沿用原有无界面权限策略。审批状态不持久化到历史，客户端
断线重连必须使用新的待审批快照，不能重放旧的回答。

### 会话内 Elicitation

`WatchElicitations { session }` 与 `RespondElicitation { session, request_id, response }`
为已有会话提供同一套 Form / URL 交互。外层 session 使用 Bas UUID；请求中的 ACP
`sessionId` / `requestId` 保留原始值，由 daemon 对照当前连接校验，不由客户端改写。
待处理快照包含原始请求、唯一回答 ID 和 `answered`；URL 已接受时保持等待状态，匹配
完成通知后进入 `completed_ids`，其中记录的是 Bas 回答 ID。

响应只消费一次，校验失败保留请求供修正。回调取消、回合取消、会话关闭、连接替换
或最后一个查看者断开会清理交互；无查看者时返回取消。旧连接的完成或取消不会影响
新连接。重连读取当前快照，不能重放旧答案或自动打开 URL。

### 可恢复归档

`SetArchived { session, archived }` 通过会话恢复锁串行化归档、启动、关闭和删除，
发布带 revision 的 `ArchiveChanged`，并返回完成操作时的完整 Session 快照。
相同值的重复请求不产生新事件，也不会隐式启动未归档的停止会话。
客户端仍按 revision 合并响应、目录和事件，不能用迟到响应覆盖更高版本。

`ListSessions` 默认只返回未归档会话，`include_archived=true` 返回完整目录。
归档先停止运行实例和正在执行的回合，再提交归档标记；不删除会话或历史。
归档必然已停止，已停止不必然归档。`GetSession`、`Attach` 和 `LoadHistory` 继续可读取记录。
归档后新 `Prompt` 返回 `FAILED_PRECONDITION`；不会通过发送或取消请求自动重启。
恢复加载原会话，成功后将取消归档与就绪状态一起提交；失败、能力不支持或提交前取消
保持原记录已归档且已停止，不创建替代会话。已知持久化故障仍拒绝修改。

GUI 在归档时保留草稿和未发队列并暂停派发，恢复本身不自动解除队列暂停。
`DeleteSession` / `bast session remove` 仍是停止 Agent 并遗忘会话的永久删除操作，
不作为旧服务器不支持归档时的兼容替代。

归档元数据与会话快照一起提交。默认 `localdb` 保留会话与历史事件，daemon 重启后可读取；
加载旧版本归档但未停止的记录时，将其规范为停止状态，不启动归档的 Agent。
`memory` 模式只保留有界内存历史，重启后丢失。Agent 继续对话还取决于其会话恢复能力，
已淘汰的内存事件不会恢复。
归档功能需更新 daemon、CLI 与 GUI；旧 daemon 返回 `UNIMPLEMENTED` 时应提示升级，
不能把新增字段的默认 `false` 当作旧服务支持归档的证据。

### 最近优先的历史窗口

`LoadHistoryWindow { session, before?, limit }` 与原有 `LoadHistory` 分开。
`before` 缺省读取最新内容；指定时只读取严格小于该游标的事件。`limit` 必须为正，
是目标数量而非硬截断：服务端沿消息、工具、压缩及终端依赖向前扩展，返回连续事件区间。
ACP message ID 按角色和会话匹配，可以跨回合；没有 ID 的内容按 reducer 的相邻流式消息规则匹配。
导入历史可以没有回合标记，不能把回合或固定事件条数当成安全切点。

响应包含：

- `session`：同一已提交快照的完整会话元数据和当前运行健康状态；
- `events`：时间顺序排列、游标连续的原始事件，payload 和 cursor 不重写；
- `context`：窗口之前仍需回放的全局展示状态（计划、用量等），以及窗口起点仍活动的原始
  `TurnStarted`；不是另一份聊天记录；
- `before`：下一次向前读取使用的排他上界，缺省表示历史已耗尽；
- `cursor`：本次快照高水位。首次读取最新窗口后从这里 `Attach`，不会遗漏加载期间的新事件。

客户端保留原始窗口事件。向前加载后以较旧窗口的 `context` 和拼接后的事件重建 reducer，
不要把较新 context 插到较旧正文之前。向前加载响应的 `cursor` 是该次请求观察到的高水位，
不能替代已打开最新窗口的实时续传游标；新事件仍由 Attach 接收。

窗口和上下文一起超过 32 MiB gRPC 上限时返回 `RESOURCE_EXHAUSTED`，
不会缩短为半条消息或静默跳过依赖。前端与后端必须一起升级，
客户端不提供旧 daemon 的完整历史兼容路径。
Memory 不能恢复已淘汰的依赖，遇到缺失内容返回 `DATA_LOSS`。
LocalDb 保存可重建的依赖索引，旧记录首次使用时可能需要构建索引；后续页面复用索引，
不为每次分页重新读取完整历史，也不在 daemon 中缓存大量完整会话。
真实依赖可能跨越整个会话，因此不能保证每个最新窗口都是小页面或具有固定加载延迟。

共享 Rust API 为 `impl Client` 成员，返回 `tonic::Status`：

```rust,ignore
let recent = client.history_window(session, None, 256, timeout).await?;
let older = client.history_window(session, recent.before, 256, timeout).await?;
```

`HistoryWindow` 从 `bas::service` 导出，支持 Clone 和 Serde。
共享加载器校验会话身份、游标顺序、连续性和分页进展。
每次 `history_window` RPC 独立超时；原始 `LoadHistory` API、daemon 与 CLI 用法不变。

实时追加使用 `bas::model::history::DependencyIndex`：按顺序 `observe(&event)`
已有 context 和窗口事件，收到新事件后先检查 `may_reference_unloaded(&event)`。
若历史尚未全部加载且该方法返回 true，应重新读取最新的依赖对齐窗口，而不是先创建孤立消息或工具；
否则再 `observe` 并应用事件。未知显式 ID 可能表示新项目，也可能引用未加载历史，
所以该检查有意保守。全局 running/idle 展示可从会话快照初始化；
context 不添加历史回合完成或错误通知，以免产生窗口外的虚构聊天条目。
`TurnStarted` 只作为不产生可见条目的状态种子，不因 `TurnFinished` 将整个长回合拉入窗口。
计划 context 保留当前插入代的首次事件及最后快照，维持 reducer 的计划顺序。

### 回放与输入边界

可回放的 ACP 更新进入历史，包括流式内容块、计划、用量与压缩。
标准 `Notice` 只向当前 Attach 订阅广播，使用 `cursor = 0`，不写入历史、
不改变会话 revision、更新时间或客户端续传游标。它可能在断线时丢失，回放不会重新弹出。
客户端先处理这种实时通知，再对可回放事件按正数 cursor 去重，以相同逻辑
处理直播和回放；文本、思考分别累积，工具更新使用 ACP 的 `ToolCall::update`。
内存历史游标过期仍返回 `DATA_LOSS`；该容量按所有更新计数，淘汰可能发生在一条消息中间。
`localdb` 通过持久事件分页恢复历史；`LoadHistory` 不承诺返回已淘汰的内存内容。
大附件也会占用历史存储。
客户端与服务端的单条 gRPC 消息上限为 32 MiB，包含 base64 开销。历史分页同时遵守
请求的事件条数和编码字节预算，达到预算时提前结束页面，并返回最后实际输出事件的
`next`。单个事件已超过上限时返回 `RESOURCE_EXHAUSTED`，不会静默跳过或截断它。

当前协议的工作目录为 UTF-8 字符串；CLI 和 daemon 都拒绝无法无损表示的路径，
包括 canonicalize 后指向非 UTF-8 目录的符号链接，不使用替换字符改写路径。
CLI 的人类可读输出会转义终端控制字符；JSON 输出保留 JSON 自身的编码语义。

这是对旧自定义消息协议的破坏性调整，daemon、CLI、GUI 应一起重新编译和更新。
旧的 ContentBlock / Delta / Message protobuf 字段已移除，事件字段号保留以避免复用。
GUI 的 `Message` 仅用于界面聚合，不属于 RPC schema。GUI 附件选择直接构造 ACP 图片和嵌入资源，
与文本一起放入 `PromptRequest.prompt`；Agent 未声明 `embeddedContext` 时，GUI 将文本文件转换为带文件名和 URI 的 ACP 文本块。
没有新增自定义上传协议。交互审批使用上文的原生权限请求与回答接口。

ACP 原始定义参见 [内容类型](https://agentclientprotocol.com/protocol/v1/content) 与
[会话配置选项](https://agentclientprotocol.com/protocol/v1/session-config-options)。
