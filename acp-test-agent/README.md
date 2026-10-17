# ACP Test Agent

一个用于测试 ACP（Agent Client Protocol）客户端的脚本化 Agent。它同时实现 ACP
v1 与 v2，没有模型、网络或真实 AI 能力：prompt 文本被当作命令，执行固定的功能演示。
命令覆盖会话生命周期、流式消息、工具调用、计划、权限、Elicitation、文件、终端、配置、
Notice、压缩、扩展方法等，便于手动验证客户端 UI 与协议处理。

项目位于主仓库的独立目录中，拥有自己的 `Cargo.toml`、`Cargo.lock` 和 workspace，
与主项目的 `src/` 完全分离；未来可以直接拆分为独立仓库。

## 快速开始

```sh
cd acp-test-agent
cargo run
```

进程通过 stdio 收发 JSON-RPC，stdout 只用于协议；诊断信息输出到 stderr。
`cargo run -- --help` 查看启动参数。

把它配置成任意 ACP 客户端的 Agent 即可。在当前 Bas 检出中，`bas` 的 Agent 目录使用
如下 `AgentSpec` 形状（`program` 指向编译产物，建议用绝对路径）：

```json
{
  "id": "acp-test-agent",
  "kind": "acp",
  "command": {
    "program": "/absolute/path/to/acp-test-agent/target/release/acp-test-agent",
    "args": []
  }
}
```

启动后在会话中发送 `/help`，会列出全部脚本命令。普通文本按 `/echo <text>` 处理。

### 启动参数

| 参数 | 默认值 | 作用 |
| --- | --- | --- |
| `--page-size <N>` | `2` | `session/list` 每页返回的会话数；用于验证分页 UI |
| `--require-auth` | 关闭 | `session/new` 前必须先 `authenticate`（v1）或 `auth/login`（v2） |
| `--protocol <both\|v1\|v2>` | `both` | 限制 Agent 提供的协议版本，用于验证客户端的降级/回退逻辑 |

认证方法 id 为 `demo-login`。`--require-auth` 只拦截新建会话，不影响 `session/load`、
`session/resume` 等已有会话操作。`--protocol v1` 时 v2-only 客户端会在 initialize 阶段
失败；`--protocol v2` 则拒绝 v1-only 客户端，便于测试客户端的协议选择与回退路径。

## 工作方式

- 启动时使用 SDK 的 `Agent::protocol_router()` 同时注册 v1 和 v2 实现；
  首个 `initialize` 请求声明的版本决定由哪个实现接管连接。
- v1 与 v2 分别实现各自的 RPC、会话更新和作用域语义。v2 的 prompt 会先返回
  `messageId` 表示已接收，再通过 `state_update` 报告 `running` / `idle`；v1 的
  `session/prompt` 在回合结束后返回 `stopReason`。
- 会话保存在进程内存中，关闭进程即丢失；同一会话的 update 会按顺序记录，供
  `session/load`（v1）或带 `replayFrom: start` 的 `session/resume`（v2）回放。
- 每个 prompt 都在连接内单独 spawn，因此 `session/cancel`、权限请求和 Elicitation
  可以在回合执行期间正常到达。

## Prompt 命令

| 命令 | 说明 | v1 | v2 |
| --- | --- | --- | --- |
| `/help` | 列出命令，并发送 available commands 更新 | ✅ | ✅ |
| `/echo <text>` | 用多个 message chunk 流式回显 | ✅ | ✅ |
| `/rich` | text、image、audio、resource link、embedded resource | ✅ | ✅（额外发送 message upsert） |
| `/thought` | 先发送 thought chunk 再回答 | ✅ | ✅（额外发送 thought upsert） |
| `/tool` | tool call 生命周期、内容、diff、raw output | ✅ | ✅（upsert + content chunk + 多文件结构 diff） |
| `/plan` | 发布执行计划 | ✅ | ✅ |
| `/permission` | 向客户端请求权限并回报结果 | ✅ | ✅ |
| `/elicit` | 发送表单 Elicitation | ✅ | ✅ |
| `/elicit-url` | 发送 URL Elicitation，并发送 `elicitation/complete` | ✅ | ✅ |
| `/fs` | 调用客户端 `fs/read_text_file` 与 `fs/write_text_file` | ✅ | 报告 v2 已移除文件回调 |
| `/terminal` | 终端演示 | ✅ 客户端托管 `terminal/*` | ✅ Agent 自有 terminal 更新 |
| `/sleep <ms>` | 等待指定毫秒数，便于测试取消 | ✅ | ✅ |
| `/config [model=fast\|deep\|on\|off\|toggle]` | 修改配置并发送 `config_option_update` | ✅ | ✅ |
| `/commands` | 重新发送 available commands 更新 | ✅ | ✅ |
| `/usage` | 发送 usage/cost 更新 | ✅ 回合响应也携带 usage | ✅ |
| `/notice` | 发送用户可见 Notice | 需要客户端声明能力 | ✅ |
| `/compact` | 发送上下文压缩 summary 与完成状态 | 需要客户端声明能力 | ✅ |
| `/info` | 更新会话标题与时间 | ✅ | ✅ |
| `/extension` | 发送自定义 `_demo/echo` 请求与 `_demo/notice` 通知 | ✅ | ✅ |
| `/fail` | 让回合以错误结束 | JSON-RPC error | `idle` 中的 error stop reason |
| `/stop <reason>` | 指定 stop reason | ✅（`error` 映射为 JSON-RPC error） | ✅（`error` 映射为 idle error） |

能力相关命令在客户端未声明对应能力时会返回说明文本，而不是伪造成功。

## 协议覆盖

### ACP v1

- 生命周期：`initialize`、`authenticate`、`logout`、`session/new`、`session/load`、
  `session/list`、`session/delete`、`session/resume`、`session/close`、`session/fork`、
  `session/set_mode`、`session/set_config_option`、`session/prompt`、`session/cancel`。
- Agent → Client：`fs/read_text_file`、`fs/write_text_file`、`terminal/create`、
  `terminal/output`、`terminal/wait_for_exit`、`terminal/kill`、`terminal/release`、
  `session/request_permission`、`elicitation/create`、`elicitation/complete`。
- 会话更新：agent/thought message chunks、tool call 与更新、plan、plan_update（客户端
  声明能力时）、available commands、current mode、config option、session info、usage、
  notice、compaction、extension 通知。

### ACP v2

- 生命周期：`initialize`、`auth/login`、`auth/logout`、`session/new`、`session/list`、
  `session/delete`、`session/resume`、`session/close`、`session/fork`、
  `session/set_config_option`、`session/prompt`、`session/cancel`。
- Agent → Client：`session/request_permission`、`elicitation/create`、
  `elicitation/complete`。v2 没有客户端文件与终端执行回调。
- 会话更新：user/agent/thought message chunks 与 upsert、`state_update`
  running/idle、tool call upsert 与 content chunk、Agent 自有 terminal update/output、
  plan update、available commands、config option、session info、usage、notice、
  compaction、extension 通知。

### 未覆盖

- `mcp/message`、MCP server 实际连接（只记录 `newSession` 携带的数量）。
- unstable providers（`providers/*`）、NES、subagent 与 session message。
- 会话持久化、真实文件内容、真实命令执行、真实模型输出。
- 音频目前只作为输出内容展示，不处理客户端音频输入。

这些是测试 Agent 的非目标，不代表客户端不需要它们；需要时可按同一命令模式增加。

## 测试与调试

```sh
cd acp-test-agent
cargo fmt --all -- --check
cargo clippy --all-targets
cargo test
```

集成测试会启动编译出的真实二进制，通过 SDK 的客户端侧建立 stdio 连接，分别覆盖：

- `tests/v1.rs`：v1 生命周期、分页、配置/模式、工具/计划/权限/Elicitation/文件/终端、
  扩展、usage/notice/compaction、load/resume/fork、取消。
- `tests/v2.rs`：v2 生命周期、prompt 接受与 idle、消息 upsert、工具/计划/权限/
  Elicitation、Agent 自有终端、扩展、usage/notice/compaction、resume replay、取消。
- `tests/auth.rs`：`--require-auth` 下两个版本的认证与登出。
- `tests/routing.rs`：`--protocol v1/v2` 的协议选择，以及 v2-only 客户端对 v1-only
  Agent 的失败路径。

手动调试时可以让 stderr 保持可见；stdout 被协议占用，不要向 stdout 写日志。

## 目录结构

```text
acp-test-agent/
  Cargo.toml
  src/
    lib.rs              路由 v1/v2 并启动 stdio 连接
    main.rs             命令行入口
    cli.rs              启动参数
    demo.rs             协议无关的命令目录、配置与资源
    demo/commands.rs    prompt 命令解析
    demo/sessions.rs    内存会话存储与分页
    demo/assets/        embedded resource 示例
    extensions.rs       `_demo/*` 自定义扩展类型
    v1.rs               v1 连接、初始化与认证
    v1/                 v1 会话、prompt、update、客户端回调
    v2.rs               v2 连接、初始化与认证
    v2/                 v2 会话、prompt、update、客户端回调
  tests/                启动真实进程的集成测试
```

## 许可证

MIT，见 [LICENSE](LICENSE)。
