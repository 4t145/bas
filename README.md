# **BASR** is a Basic Agent Service in Rust

BASR 的目的是为linux系统（或者可能更多）在各种各样的环境下提供一个基础的agent服务能力。

可能有以下应用场景
1. 比如在系统的装机阶段，或者在用户缺少python/node环境或者相关依赖的时候，有或者是缺少图形界面的时候，能够获得一定的AI能力。
2. 有一个系统级别的ai唤起流程，可以通过桌面/语音唤起配置的agent provider来为用户完成任务。

项目分为如下几个部分
1. 守护进程`basrd`，负责管理agent会话，提供服务供客户端使用，发出通知等交互内容
2. 命令行管理工具`basrctl`，能够通过命令行来管理`basr`服务
3. GUI/TUI客户端的配套。其中tui要保持简单与高度可用，能够在tty等环境下正常工作。（这个任务优先级不高）

出于可用性的考虑，应当尽量减少动态库依赖。比如tls使用rustls, 使用musl静态链接编译等。

## 开发环境

开发环境由 Nix flake 提供，Rust 工具链版本固定在 `rust-toolchain.toml`，`flake.lock` 锁定 nixpkgs 与 rust-overlay 的版本。

```sh
nix develop        # 进入开发 shell
cargo check        # 在 shell 内构建
```

安装了 direnv 的用户，首次进入仓库执行一次 `direnv allow`，之后 `cd` 进目录会自动加载并缓存环境（需要 [nix-direnv](https://github.com/nix-community/nix-direnv)，`.envrc` 会自动引导）。

shell 内提供 `rustc`、`cargo`、`clippy`、`rustfmt`、`rust-analyzer` 与 `rust-src`（供 rust-analyzer 解析标准库源码）。

升级工具链：修改 `rust-toolchain.toml` 的 `channel`（`.envrc` 已 `watch_file` 该文件，direnv 会自动重载）；若该版本不在当前 rust-overlay 快照中，再运行 `nix flake update rust-overlay`。

## 目录结构

```text
🦀Cargo.toml
  +--📄proto/basr/v1/basr.proto      gRPC 接口定义（basr.v1.Basr）
  +--🦀build.rs                      protoc-bin-vendored + tonic-prost-build 生成代码
  +--🦀src/lib.rs                    共享实现：模型 / 后端 / 会话 / 服务 / 通知
  |   +--🦀model.rs                  d 层：与后端无关的领域类型
  |   +--🦀agent.rs                  后端抽象与驱动
  |   |   +--🦀embedded.rs           进程内 agent（当前为 echo 占位）
  |   |   +--🦀acp.rs                外部 agent（Agent Client Protocol 客户端）
  |   +--🦀session.rs                会话生命周期
  |   |   +--🦀conversation.rs       单个会话：回放窗口 + 事件广播
  |   |   +--🦀attachment.rs         attach/detach 与游标续传
  |   |   +--🦀history.rs            有界历史环形缓冲
  |   |   +--🦀registry.rs           会话注册表与后端目录
  |   +--🦀service.rs                gRPC 服务端/客户端与 pb↔model 转换
  |   +--🦀notify.rs                 D-Bus 桌面通知与完成监听
  |   +--🦀config.rs                 环境变量配置
  +--🦀src/bin/basrd/                守护进程
  +--🦀src/bin/basrctl/              命令行管理工具
```

## 构建与运行

```console
$ cargo build --release
$ ./target/release/basrd                       # 默认监听 127.0.0.1:7337
$ ./target/release/basrctl agents
$ ./target/release/basrctl session create --title demo
```

构建不需要系统安装 `protoc`：`build.rs` 使用 `protoc-bin-vendored` 提供的编译器。

## 配置

全部通过环境变量读取，systemd 单元无需可写配置目录；命令行同名参数可覆盖。

| 环境变量 | 默认值 | 说明 |
| --- | --- | --- |
| `BASRD_LISTEN` | `127.0.0.1:7337` | gRPC 监听地址（默认仅回环） |
| `BASRD_AGENTS` | `embedded` | 后端目录，见下 |
| `BASRD_NOTIFY` | `auto` | 桌面通知策略：`auto` / `always` / `never` |
| `BASRD_HISTORY` | `4096` | 每个会话保留的可回放事件数 |
| `BASRD_LOG_BACKEND` | `auto` | 日志后端：`auto` / `stdout` / `journald` |
| `RUST_LOG` | `info` | 日志过滤表达式 |

`BASRD_AGENTS` 用 `;` 分隔条目，条目形如 `id`、`id=kind` 或 `id=kind:命令行`：

```console
$ BASRD_AGENTS='embedded;opencode=acp:opencode acp;gemini=acp:gemini --acp' basrd
```

`kind` 取 `embedded` 或 `acp`；外部后端必须给出命令行（按空白切分，不支持引号）。

## basrctl

```console
$ basrctl agents                                  # 列出可用后端
$ basrctl session create --agent embedded --title demo
$ basrctl session list
$ basrctl session show <id>
$ basrctl prompt <id> 帮我看看这个目录
$ basrctl attach <id> --replay 20                 # 回放最近 20 条并持续跟随；Ctrl-C 只是 detach
$ basrctl attach <id> --cursor 42 --replay 50     # 按游标续传
$ basrctl history <id> --after 42 --limit 50      # 翻页读取历史
$ basrctl cancel <id>
$ basrctl session rename <id> 新标题
$ basrctl session remove <id>
```

`--server` 可指定守护进程地址，默认 `127.0.0.1:7337`。

## 架构要点

### d 层消除后端差异

所有后端都产出 `agent::Update`，会话层再包装成带游标和时间戳的 `model::Event`。
`service` 负责 `model` 与 protobuf 的互转，因此 TUI/GUI 拿到的永远是同一套类型，
不需要知道会话背后是进程内 agent 还是 ACP 外部进程。

### 会话、attach 与游标

- 会话 id 为 UUID，元数据包含后端 id、发起用户与主机、工作目录、标题、状态。
- `detach` 只丢弃 `Attachment`；agent loop 继续运行，事件继续进入会话的回放窗口。
- 重新 `attach` 时先按请求回放最近 N 条**可回放事件**，随后转入直播。
- 游标从 1 开始单调递增且不复用；客户端把最后看到的游标传回 `Attach` / `LoadHistory`
  即可续传。`Cursor::START`（0）表示“第一条之前”。
- 流式 `Delta` 只用于直播，不写入历史：提交后的 `Message` 携带同一 `message.id`，
  客户端据此用完整消息替换已流式渲染的内容。
- 客户端落后超过广播容量时收到 `DATA_LOSS`，按最后游标重新 attach 即可恢复。

### 持久化

按设计文档，真正的消息持久化属于被调用的 agent 侧；守护进程只保留有界回放窗口
（`BASRD_HISTORY`），用于 attach 播报与游标翻页。

### systemd 与桌面通知

- 日志：`BASRD_LOG_BACKEND=journald` 时直接写 journald（`tracing-journald`）。
- 就绪与看门狗：`sd_notify` 发送 `READY=1` / `STOPPING=1`，`WatchdogSec=` 时按半个
  周期 ping。参考单元文件见 `contrib/basrd.service`。
- 通知：会话每个回合结束时通过 `org.freedesktop.Notifications` 提醒；没有会话总线
  （如 headless 环境）时自动关闭。

## 当前状态

已可端到端运行（`basrd` + `basrctl`），并有白箱测试覆盖模型、后端目录、会话回放、
游标分页与 protobuf 往返。

仍在演进的部分：

- **嵌入式 agent**：`agent/embedded.rs` 目前是 echo 占位实现，用于在真实 agent 落地前
  打通全链路；替换 `Echo` 即可，会话层无需改动。
- **ACP 后端**：已实现 `initialize`、`session/new`、`session/prompt`、`session/cancel`、
  文件读写回调与 `session/request_permission`；`terminal/*`、plan、mode、usage 等
  尚未映射到模型。`set_title` 在 ACP 下返回不支持（协议只允许 agent 上报标题）。
- **权限**：默认 `LocalHost` 按固定策略放行；交互式审批需要实现自定义 `Host`，
  把 `PermissionRequest` 路由给已 attach 的前端。
- **传输**：当前为回环 TCP，Unix domain socket 尚未实现。
