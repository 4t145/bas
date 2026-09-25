# 本机运行：GUI + basrd + pi

在 Linux 仓库目录下运行，使用启用了 flakes 的 Nix。入口自动准备固定版本的
Rust、图形库、pi 和 pi-acp，不需要另外安装 Node 或 npm 包。
这是开发 / 测试专用环境，不是产品安装流程；Pi 固定依赖不进入 BASR 安装包，
也不安装到用户 profile。日常使用请按 [安装说明](install.md) 独立安装和管理外部工具。

## 启动图形界面

在仓库根目录的 `.env` 中配置：

```dotenv
DEEPSEEK_API_KEY=your-api-key
```

已有 `.env` 时直接使用，不要覆盖它。非空的同名环境变量优先于文件。
脚本使用 `python-dotenv` 解析，不执行 `.env` 中的 shell 命令，也不展开变量引用。

```sh
./scripts/dev-local

# 直接打开按 workspace 分组的窗口（默认是最小弹窗）
./scripts/dev-local gui --window

# 让新会话默认使用你的项目目录
./scripts/dev-local gui --cwd /path/to/project
```

入口先编译 `basrd`、`basrctl` 和 `basrw`，再启动监听 `127.0.0.1:7337` 的
basrd，注册 `pi` 后端，最后打开已连接的 GUI。在输入框中输入任务并按 Enter 即可开始，
首次发送会自动创建 thread。Ctrl+Shift+M 在最小弹窗和 workspace 窗口之间切换。
首次创建会话时才启动 pi。初次准备环境可能需要下载依赖，后续会复用缓存。

默认工作目录是新建的 `.test-results/local.*/work/`，适合先试文件读写。
本机 pi 使用当前用户的权限，这个工作目录不是沙箱。需要虚拟机隔离时使用
[系统测试入口](testing.md#真实-pi-与-deepseek)。

这个一体化入口用于一次本地试运行：**关闭 GUI 或在启动终端按 Ctrl-C，会一起停止
它启动的 basrd 和 pi 会话**。文件及日志保留，会话列表随 basrd 退出而清空。
若希望关掉 GUI 后 Agent 继续运行，使用下面的分开启动方式。

端口已占用时入口会报错，不会接管现有 basrd。可以指定其他端口，或让系统分配：

```sh
./scripts/dev-local gui --port 7338
./scripts/dev-local gui --port 0
```

修改源码后重新执行会增量编译。确定现有二进制已更新时，可加 `--no-build`。

## 分开启动服务与客户端

终端一：

```sh
./scripts/dev-local daemon
```

终端二，在仓库目录：

```sh
# GUI 可独立退出，终端一的服务继续运行
nix develop .#gui -c cargo run --features gui --bin basrw -- \
  --server 127.0.0.1:7337

# 也可以直接使用已经构建好的 CLI
./target/debug/basrctl --server 127.0.0.1:7337 agents
./target/debug/basrctl --server 127.0.0.1:7337 session create \
  --agent pi --cwd /path/to/project --title local
./target/debug/basrctl --server 127.0.0.1:7337 prompt <session-id> '你好'
./target/debug/basrctl --server 127.0.0.1:7337 attach <session-id> --cursor 0
```

若指定了其他端口，使用启动终端打印的地址。终端一按 Ctrl-C 会回收该服务和后端进程。

## 自动验证真实后端

```sh
./scripts/dev-local smoke
```

这会调用真实 DeepSeek API 并产生 API 用量。测试使用自动分配的本机端口，
在新的测试目录中生成随机文本 `input.txt`，通过 basrctl 让 pi 读取它、写出
大写内容到 `output.txt`、读回检查，最后回复 `BASR_PI_OK`。
检查回合正常结束、工具完成事件和输出内容后，删除测试会话并停止全部测试进程。
测试不会以你的项目目录作为工作目录；即使传了 `--cwd`，smoke 也使用新建的测试目录。

## 配置与排查

修改启动脚本后，可在本地 Nix shell 中运行不调用模型的检查：

```sh
nix develop .#local -c python tests/local_runner.py -v
```

这使用占位密钥检查配置解析、真实 pi 的 ACP 初始化与进程回收，不发送模型请求。
直接使用 `.#local` 需要相关 Nix 文件已被 Git 跟踪；`scripts/dev-local` 会自动生成
包含未跟踪源码、排除密钥与构建输出的快照，因此日常启动不需要改动 Git 暂存区。

- 复用虚拟机测试中固定的 pi 0.75.4、pi-acp 0.0.32 和 DeepSeek 模型配置。
  默认模型为 `deepseek-flash`，thinking 和自动重试关闭，单次回复上限为 2048 tokens。
- 每次运行的 pi 配置位于 `.test-results/local.*/pi-agent/`，通过
  `PI_CODING_AGENT_DIR` 指定，不修改 `~/.pi`。项目自身的 `.pi/settings.json`
  仍可能覆盖全局设置。
- 密钥只通过运行时环境传给 basrd 和 pi，不写入模型配置、不传给 GUI 或 CLI。
  `.env`、构建输出和测试日志不会进入 Nix 源码快照。
- `PI_OFFLINE=1` 关闭 pi 启动时的更新检查；模型请求仍然需要联网。
- 日志位于启动终端打印的本次目录：`daemon.log`、`gui.log`（GUI 模式）和
  `history.log`（smoke 模式）。检查本次目录，避免误看之前的运行记录。
- 连接失败先检查端口与 `daemon.log`；创建会话失败检查 pi 启动日志；模型调用失败
  检查密钥、账户额度及网络连接。

后端使用 [pi-acp](https://github.com/svkozak/pi-acp) 将 BASR 的 ACP 请求接到 pi。
认证方式参见 [pi 的 provider 文档](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md)。
