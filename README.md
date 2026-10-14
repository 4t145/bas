# **Bas** is a Basic Agent Service

Bas 的目的是为linux系统（或者可能更多）在各种各样的环境下提供一个基础的agent服务能力。

从旧版本升级前请阅读 [破坏性改名与手动迁移](docs/rename-migration.md)。

可能有以下应用场景
1. 比如在系统的装机阶段，或者在用户缺少python/node环境或者相关依赖的时候，有或者是缺少图形界面的时候，能够获得一定的AI能力。
2. 有一个系统级别的ai唤起流程，可以通过桌面/语音唤起配置的agent provider来为用户完成任务。

项目分为如下几个部分
1. 守护进程 Bas（`bas`），负责管理agent会话，提供服务供客户端使用，发出通知等交互内容
2. 终端客户端 BasT（`bast`），能够通过命令行来管理 Bas 服务
3. GUI/TUI客户端的配套。BasG（`basg`）使用 GPUI Kit 管理会话并与 Agent 对话；TUI 要保持简单与高度可用，能够在 tty 等环境下正常工作。（TUI 优先级不高）

出于可用性的考虑，应当尽量减少动态库依赖。比如tls使用rustls, 使用musl静态链接编译等。

## 开发环境

开发环境由 Nix flake 提供，Rust 工具链版本固定在 `rust-toolchain.toml`，`flake.lock` 锁定 nixpkgs 与 rust-overlay 的版本。

```sh
git submodule update --init --recursive  # 初始化固定版本的 gds-uim 宏
nix develop        # 进入开发 shell
cargo check        # 在 shell 内构建
```

首次克隆也可使用 `git clone --recurse-submodules`。`vendor/gds-uim` 是尚未发布的
声明式 UI 宏；Cargo、Nix 打包与 CI 均使用同一份 submodule 源码，不在构建时更新版本。
来源、许可证状态和更新规则见 [源码依赖说明](vendor/README.md)。

安装了 direnv 的用户，首次进入仓库执行一次 `direnv allow`，之后 `cd` 进目录会自动加载并缓存环境（需要 [nix-direnv](https://github.com/nix-community/nix-direnv)，`.envrc` 会自动引导）。

shell 内提供 `rustc`、`cargo`、`clippy`、`rustfmt`、`rust-analyzer` 与 `rust-src`（供 rust-analyzer 解析标准库源码）。

升级工具链：修改 `rust-toolchain.toml` 的 `channel`（`.envrc` 已 `watch_file` 该文件，direnv 会自动重载）；若该版本不在当前 rust-overlay 快照中，再运行 `nix flake update rust-overlay`。

## 测试

```sh
nix develop -c cargo test --all-targets  # 日常 Rust 测试
./scripts/test-system check             # 构建、Rust 测试、QEMU 系统测试
./scripts/test-system shell             # 进入虚拟机 shell，自动启动 bas
./scripts/test-system driver            # Python 测试驱动控制台
```

系统测试使用 Linux + Nix，在 QEMU 中运行独立的 NixOS 系统，覆盖 systemd、
journald、D-Bus 通知、CLI 和 ACP 进程。首次运行需要下载构建依赖及系统组件，
没有 KVM 时使用较慢的软件模拟。成功报告位于 `.test-results/system/`。

虚拟机使用 embedded echo 和受控 ACP fixture，不预装外部 Agent、不读取凭据，
也不调用真实模型。Pi 等后端通过产品内的 ACP Registry 按需安装。

操作步骤、日志位置和新增测试的方法见 [测试环境说明](docs/testing.md)。

## 目录结构

```text
🦀Cargo.toml
  +--📄proto/bas/v1/bas.proto        gRPC 接口定义（bas.v1.Bas）
  +--🦀build.rs                      protoc-bin-vendored + tonic-prost-build 生成代码
  +--🦀src/lib.rs                    公共库：领域模型与客户端协议支持
  |   +--🦀model.rs                  会话生命周期、事件信封与 ACP 类型导出
  |   +--🦀service.rs                gRPC 信封、ACP JSON 编解码与客户端连接
  |       +--🦀defaults.rs           客户端与守护进程共用的地址、回放默认值
  +--🦀src/bin/bas/                  守护进程内部实现
  |   +--🦀main.rs                   组装与启动
  |   +--🦀agent.rs                  后端抽象与驱动
  |   |   +--🦀embedded.rs           进程内 agent（当前为 echo 占位）
  |   |   +--🦀acp.rs                外部 agent（Agent Client Protocol 客户端）
  |   +--🦀session.rs                会话生命周期
  |   |   +--🦀conversation.rs       单个会话：回放窗口 + 事件广播
  |   |   +--🦀attachment.rs         attach/detach 与游标续传
  |   |   +--🦀history.rs            有界历史环形缓冲
  |   |   +--🦀registry.rs           会话注册表与后端目录
  |   +--🦀storage.rs                memory/localdb 会话存储
  |   +--🦀service.rs                基于会话注册表的 gRPC 服务端实现
  |   +--🦀notify.rs                 D-Bus 桌面通知与完成监听
  |   +--🦀config.rs                 启动参数、环境变量与后端目录配置
  |   +--🦀system.rs                 进程生命周期与 systemd 集成
  |   +--🦀tracing.rs                日志后端
  +--🦀src/bin/bast/                 命令行管理工具
  +--🦀src/bin/basg/                 GPUI Kit 图形客户端（gui feature）
```

公共库导出 `bas::acp`（agent-client-protocol 3.3.0 的 v1 schema）、`bas::acp_v2`（展示模型）、`bas::model` 与 `bas::service`。后端驱动、会话运行时、通知和
守护进程配置由 `bas` 的私有模块管理；客户端通过 gRPC 使用它们。

## 构建与运行

本机开发分别启动 daemon 和客户端；外部 Agent 通过 GUI 的 ACP Registry 目录
或 `bast agent install` 安装，认证使用 Agent 原生流程，见 [安装说明](docs/install.md)。
GUI 源码运行方式见 [本机启动](docs/gui.md#本机启动)。

```console
$ cargo build --release
$ ./target/release/bas                         # 默认监听 127.0.0.1:7337
$ ./target/release/bast agents
$ ./target/release/bast session create --title demo
```

构建不需要系统安装 `protoc`：`build.rs` 使用 `protoc-bin-vendored` 提供的编译器。

## 安装 Bas

Linux / NixOS 在仓库根目录运行：

```sh
make install-bas-service
bast agents
```

只安装 `bas`、`bast` 并启用 systemd 用户服务；不创建或修改用户配置。
只安装可执行程序可使用 `make install-bas`。Codex、OpenCode、Pi 等外部后端可在 GUI 的 Agent 目录或 `bast agent install` 中按需安装，
也可沿用用户独立安装的程序和原生登录状态。
旧 `--with-codex` 已移除，已有捆绑包和配置不会被自动删除或改写。
官方安装链接、路径配置、迁移及服务管理见 [安装与后端配置](docs/install.md)。

## 配置

应用外观和 ACP 后端统一保存在 `bas/settings.json`，可在设置窗口中编辑。
服务监听、权限及容量等启动参数仍通过命令行或环境变量设置，命令行同名参数优先。

| 环境变量 | 默认值 | 说明 |
| --- | --- | --- |
| `BAS_LISTEN` | `127.0.0.1:7337` | gRPC 监听地址（默认仅回环） |
| `BAS_AGENTS` | 未设置 | 简写后端目录，设置后覆盖 JSON 配置，见下 |
| `BAS_SETTINGS_FILE` | 用户配置目录下的 `bas/settings.json` | GUI 与 daemon 共用配置；Linux 默认 `~/.config`，跨平台目录见 [GUI 说明](docs/gui.md#统一设置中的外观偏好) |
| `BAS_AGENTS_FILE` | 未设置 | 显式兼容旧版独立 agent 数组；设置后覆盖统一文件的 agent 目录 |
| `BAS_NOTIFY` | `auto` | `auto` 允许无桌面运行，`always` 要求通知服务可用，`never` 禁用 |
| `BAS_HISTORY` | `4096` | 每个会话的内存历史窗口容量；localdb 的磁盘历史不受此容量限制 |
| `BAS_STORAGE` | `localdb` | 会话存储：`localdb`（默认 SQLite） / `memory`（显式关闭持久化） |
| `BAS_LOCALDB_PATH` | 用户状态目录下的 `bas/sessions.sqlite3` | `localdb` 的数据库文件路径，`memory` 模式不能设置 |
| `BAS_BUFFER` | `1024` | 后端更新队列与直播广播容量，必须大于 0 |
| `BAS_STARTUP_TIMEOUT` | `30` | 后端启动超时（秒），必须大于 0 |
| `BAS_PERMISSIONS` | `deny` | 工具权限申请的固定审批策略：`deny` / `allow` |
| `BAS_LOG_BACKEND` | `auto` | 日志后端：`auto` / `stdout` / `journald` |
| `RUST_LOG` | `info` | 日志过滤表达式 |

默认启动会将新会话存入用户状态目录；可显式覆盖数据库位置：

```console
$ bas --storage localdb --localdb-path "$HOME/.local/state/bas/sessions.sqlite3"
```

仅需临时会话时使用 `--storage memory`。从旧的仅内存版升级不会迁移已有进程内会话；
重启旧 daemon 前请先处理仍在运行的会话。

数据库需要长期保留会话内容，应限制文件及目录访问权限并安排备份。持久化记录不等于所有外部
Agent 都能恢复执行上下文；只有后端支持 ACP `session/load` 或 `session/resume` 且成功完成恢复，
才允许在旧会话继续发送 prompt。重启时未完成的 turn 会记录为中断，不会自动重发工具操作。
当前内嵌 `embedded` 后端仅是无状态 echo，不提供真正的模型对话上下文。

`BAS_AGENTS` 用 `;` 分隔条目，条目形如 `id`、`id=kind` 或 `id=kind:命令行`：

```console
$ BAS_AGENTS='embedded;opencode=acp:opencode acp;gemini=acp:gemini --acp' bas
```

`kind` 取 `embedded` 或 `acp`；外部后端必须给出命令行（按空白切分，不支持引号）。
未设置简写目录时，读取显式 `--agents-file` 兼容文件，否则读取 `--settings-file` /
`BAS_SETTINGS_FILE` 或默认 `settings.json` 的 `daemon.agents`。
JSON 支持完整参数数组，示例见 [contrib/settings.json](contrib/settings.json)。
文件目录每秒热更新，新建会话采用新配置，已有会话保持原后端，不需要重启。
默认统一文件不存在时只读导入原 `appearance.json` 和 `agents.json`；设置窗口首次保存后写入
统一文件，保留旧文件。两者也都不存在时使用 `embedded`。

## BasG

图形客户端与 `bast` 共用服务接口。Linux / NixOS 安装：

```sh
make install-basg
basg --server 127.0.0.1:7337
```

安装包包含图形运行库与桌面入口，bas 服务单独运行。`make install` 同时安装 daemon、CLI 和 GUI；
启用用户服务使用 `make install-bas-service`。优先使用 `make install-bas` / `make install-basg`：
源码快照包含未跟踪源码和已初始化的子模块，排除 `.git`、`.delta`、`.env` 等元数据。
`make help` 列出安装目标与传递 Nix 参数的方式。源码均加入 Git 后也可使用
`nix profile add "git+file://$PWD?submodules=1#basg"`；开发运行使用
`nix develop .#gui -c cargo run --features gui --bin basg`，此前需在物理 checkout
执行 `git submodule update --init --recursive`。
支持会话创建、搜索、切换、重命名、可恢复归档、流式对话与停止回合。默认构建不启用图形依赖。
默认打开最小输入弹窗，Ctrl+Shift+M 展开 workspace / thread 窗口，并自动跟随 DMS 的 Zed 主题。
应用自己的字体、字号、行高、透明度和主题偏好保存在统一 JSON 的 `appearance` section；
`--settings-file` 可指定位置。用 `Ctrl+,` 打开独立设置窗口，外观按“主题 / 字体 / 布局”分组。
有效外观修改自动保存并热更新所有窗口，非法配置不会覆盖当前外观。
Agent 服务页提供官方 ACP Registry 目录，可搜索、安装、更新、登录和移除托管后端；
聊天 Agent 菜单中的“安装 Agent…”打开同一目录。自定义程序仍可编辑路径和参数。
使用方式和测试命令见 [GUI 说明](docs/gui.md)。

第一方代码保持 MIT；UI 使用许可兼容的 GPUI 组件，不复制或链接 Zed 的 GPL 上层面板。
GPUI 组件使用正式发布包，不维护本地源码覆盖；未发布的 `gds-uim` 宏以独立
submodule 固定版本，其许可证状态见上述源码依赖说明。版本基线、验证结果和待处理问题见
[原版 UI 依赖基线](docs/upstream-ui.md)。

## BasT

```console
$ bast agents                                  # 列出可启动的已配置后端
$ bast agent catalog                           # 浏览可安装和已配置后端
$ bast agent install codex-acp                  # 在 daemon 主机按需安装并注册
$ bast agent login codex-acp                    # 使用后端原生认证
$ bast agent check codex-acp                    # 检查握手与会话创建
$ bast session create --agent embedded --title demo
$ bast session list
$ bast session show <id>
$ bast prompt <id> 帮我看看这个目录
$ bast attach <id> --replay 20                 # 回放最近 20 条并持续跟随；Ctrl-C 只是 detach
$ bast attach <id> --cursor 42 --replay 50     # 按游标续传
$ bast history <id> --after 42 --limit 50      # 翻页读取历史
$ bast cancel <id>
$ bast session rename <id> 新标题
$ bast session archive <id>                   # 隐藏但保留会话与历史
$ bast session list --all                     # 包括已归档会话
$ bast session restore <id>                   # 恢复到普通列表
$ bast session remove <id>                    # 永久删除：停止 Agent 并遗忘会话
```

`--server` 可指定守护进程地址，默认 `127.0.0.1:7337`。
归档不取消正在执行的回合，但归档期间不能发起新回合。会话和归档状态仍保存在
daemon 内存中，受现有历史容量限制；归档不提供跨 daemon 重启的磁盘持久化。

## 架构要点

### 复用 ACP 定义

会话交互直接使用 `bas::acp` 的请求、内容、工具、配置选项与更新类型。
gRPC 用 `AcpPayload` 承载 SDK 的 JSON；Bas 只定义会话管理、回合接收确认和游标信封。
外部 ACP 驱动直接转发更新，内置 echo 后端也产生同样的 ACP 更新。
GUI 使用 Agent 返回的 `configOptions` 构建选项控件，并展示图片、嵌入文件和工具结果。
计划、上下文用量、累计费用、通知和上下文压缩也有对应界面，
逐项支持范围见 [ACP 展示元素](docs/gui.md#acp-展示元素)。
类型映射、客户端示例和兼容性说明见 [RPC 接口](docs/rpc.md)。

### 会话、attach 与游标

- 会话 id 为 UUID，元数据包含后端 id、发起用户与主机、工作目录、标题、状态。
- `detach` 只丢弃 `Attachment`；agent loop 继续运行，事件继续进入会话的回放窗口。
- 无游标的 `attach` 先回放最近 N 条**可回放事件**，随后转入直播；指定游标时回放其后
  全部保留事件，`--replay` 不限制续传数量。历史快照和直播订阅在同一锁内建立。
- 游标从 1 开始单调递增且不复用；客户端把最后看到的游标传回 `Attach` / `LoadHistory`
  即可续传。`Cursor::START`（0）表示“第一条之前”。
- ACP 内容块、工具局部更新和配置更新都写入历史；直播与回放消费相同的事件。
  客户端按游标去重、按 ACP 语义累积内容，不再依赖独立的 Delta / Message 协议。
- 标准 ACP `Notice` 仅实时广播，游标为 0；不写历史，也不改变最后可回放游标。
- 客户端落后超过广播容量时收到 `DATA_LOSS`，可以按最后游标重新 attach。该游标之后
  的历史若已淘汰，续传仍返回 `DATA_LOSS`；省略游标可重新读取当前保留窗口。
- 同一会话只允许一个活跃回合；忙碌时提交返回 `FAILED_PRECONDITION`。后端故障产生
  可回放的 `Failed` 事件，关闭流，已停止或失败的会话不会被迟到事件重新激活。

### 持久化

按设计文档，真正的消息持久化属于被调用的 agent 侧；守护进程只保留有界回放窗口
（`BAS_HISTORY`），用于 attach 播报与游标翻页。

### systemd 与桌面通知

- 日志：`BAS_LOG_BACKEND=journald` 时直接写 journald（`tracing-journald`）。
- 就绪与看门狗：`sd_notify` 发送 `READY=1` / `STOPPING=1`，`WatchdogSec=` 时按半个
  周期 ping。收到退出信号后关闭会话、后端和订阅，在 5 秒清理期限内退出。参考单元文件
  用户服务模板见 `contrib/bas-user.service`，安装与管理见 [安装说明](docs/install.md)。
- 通知：会话每个回合结束时通过 `org.freedesktop.Notifications` 提醒；没有会话总线
  （如 headless 环境）时自动关闭。

## 当前状态

已可端到端运行（`bas` + `bast`），并有白箱测试覆盖模型、后端目录、会话回放、
游标分页与 protobuf 往返。

会话顺序、回合互斥、任务回收、ACP 故障和配置覆盖已补充回归测试，设计与接口变化见
[Bas 框架修订](docs/design/bas-review.md)。可运行 `cargo test --all-targets` 验证，其中
进程集成测试需要绑定本机回环端口，ACP 进程测试在 Unix 上使用一个受控的 shell 协议端。

仍在演进的部分：

- **嵌入式 agent**：`src/bin/bas/agent/embedded.rs` 目前是 echo 占位实现，用于在真实 agent 落地前
  打通全链路；替换 `Echo` 即可，会话层无需改动。
- **ACP 后端**：已实现初始化、创建会话、prompt、cancel、配置选项和模式设置，
  文件读写及权限回调直接复用 ACP 类型。内容、工具、plan、commands、mode、usage、
  compaction 等更新通过 RPC 与历史传输，Notice 仅实时广播。
  GUI 展示 Zed 终端扩展；客户端托管的 `terminal/*` 执行回调尚未实现。
  支持内嵌文件附件、剪贴板图片与标准 ACP 交互审批。
- **权限**：默认拒绝工具权限申请；`--permissions allow` 可开启自动批准。该策略不隔离
  后端进程的文件访问。GUI 连接审批订阅后由用户选择原生允许／拒绝选项；没有审批客户端
  的会话行为及请求取消规则见 [GUI 说明](docs/gui.md#数据与验证)。
- **传输**：当前为回环 TCP，Unix domain socket 尚未实现。
