# RPC 与 ACP 类型

BASR 的会话交互复用 `agent-client-protocol` 2.x 提供的 schema。
当前后端协商的是 ACP v1，Rust 客户端统一通过 `basr::acp` 使用该 schema；
SDK 的主版本与线上协议版本是两个概念。

gRPC 保留会话管理与后台运行所需的信封。`AcpPayload.json` 是 UTF-8 JSON bytes，
直接由 ACP SDK 的 Serde 实现编解码，不在 protobuf 中再定义一套 ContentBlock、
ToolCall、ConfigOption 或 StopReason，也不经过会把 JSON 整数转为 double 的 Struct。
SDK 已知字段、annotations 和 `_meta` 保留原有语义；对未知字段和无效可选项的处理遵循 SDK。

| gRPC 接口或字段 | ACP 类型 / BASR 职责 |
| --- | --- |
| ListAgents、CreateSession、ListSessions、GetSession、DeleteSession | 后端选择、进程与会话生命周期 |
| Session.initialization | `InitializeResponse`，包含 Agent 能力、实现信息与认证方法 |
| Session.acp | `NewSessionResponse`，包含当前 modes、configOptions 和初始扩展元数据 |
| Session.available_commands | 最新 `AvailableCommandsUpdate`，不受事件历史裁剪影响 |
| Prompt 请求 | `PromptRequest`，完整 prompt 内容与 `_meta` |
| Prompt 返回 | `TurnAccepted`，确认后台回合已接收 |
| TurnFinished.response | 完整 `PromptResponse`，包括 stopReason 和 `_meta` |
| TurnFinished.error、Failed | BASR 后端故障信息 |
| Cancel 请求 | `CancelNotification` |
| SetConfigOption | `SetSessionConfigOptionRequest` / `SetSessionConfigOptionResponse` |
| SetMode | `SetSessionModeRequest` / `SetSessionModeResponse` |
| Event.acp_update | `SessionUpdate`，包含内容块、工具局部更新、计划、命令、选项、用量和标题更新 |
| Attach、LoadHistory | 带游标的有界历史与实时订阅 |
| SetForeground | 双向流报告前台会话，逐条确认；流断开自动释放状态 |
| SetTitle | BASR 本地会话标题 |

客户端始终使用 BASR 的公开 session UUID。后端驱动负责在发给 Agent 的请求中
换成 Agent 的 sessionId，返回的 `Session.acp.sessionId` 也使用公开 UUID。
权限与文件回调在 daemon 内使用 ACP 请求及权限结果类型；文件操作和固定权限策略
仍由 daemon 执行。客户端托管的终端执行能力仍未声明；初始化时通过
`clientCapabilities._meta.terminal_output = true` 接收
[Zed 的展示用终端扩展](https://github.com/zed-industries/zed/blob/main/crates/agent_servers/src/acp.rs)。
工具更新的 `_meta.terminal_info`、`terminal_output`、`terminal_exit` 原样进入历史，
GUI 按终端 ID 累积输出并渲染，`rawOutput` 作为没有可展示内容时的兜底。
此能力在初始化时协商，更新 daemon 后应新建 session。这不是一个代理全部 ACP 方法的通用网关。

Rust 客户端示例：

```rust,no_run
use basr::{acp, service::Client};

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
`basr::service::acp_error` 解码原始 code、message、data。

所有 ACP 更新都进入历史，包括流式内容块。客户端按 cursor 去重后，以相同逻辑
处理直播和回放；文本、思考分别累积，工具更新使用 ACP 的 `ToolCall::update`。
游标过期仍返回 `DATA_LOSS`；历史容量现在按所有更新计数，淘汰可能发生在一条消息中间。
`LoadHistory` 不承诺返回已淘汰内容。大附件也会占用历史内存。
客户端与服务端的单条 gRPC 消息上限为 32 MiB，包含 base64 开销；历史页同样受此限制，
含大文件时应减小分页条数。

这是对旧自定义消息协议的破坏性调整，daemon、CLI、GUI 应一起重新编译和更新。
旧的 ContentBlock / Delta / Message protobuf 字段已移除，事件字段号保留以避免复用。
GUI 的 `Message` 仅用于界面聚合，不属于 RPC schema。GUI 附件选择直接构造 ACP 图片和嵌入资源，
与文本一起放入 `PromptRequest.prompt`；Agent 未声明 `embeddedContext` 时，GUI 将文本文件转换为带文件名和 URI 的 ACP 文本块。
没有新增自定义上传协议。交互审批尚未实现。

ACP 原始定义参见 [内容类型](https://agentclientprotocol.com/protocol/v1/content) 与
[会话配置选项](https://agentclientprotocol.com/protocol/v1/session-config-options)。
