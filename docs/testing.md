# 测试环境

日常代码测试运行在开发环境中；涉及服务管理、进程退出和桌面通知的测试运行在
QEMU 的 NixOS 虚拟机中。Nix 依赖由 `flake.lock` 固定，Rust 依赖由 `Cargo.lock` 固定。

## 日常测试

本机直接测试 GUI 和真实 pi 后端可使用 `./scripts/dev-local`，自动验证使用
`./scripts/dev-local smoke`；二者读取仓库根目录 `.env` 中的 DeepSeek 密钥。
详见 [本机运行说明](local.md)。以下虚拟机入口继续用于系统集成与隔离测试。

在仓库根目录运行：

```sh
nix develop -c cargo test --all-targets
```

已经进入开发 shell 时可以直接运行 `cargo test --all-targets`。这一层包括
Rust 单元测试、gRPC 和进程集成测试，不启动虚拟机。

默认目标不包含可选的 GUI。完整本地验收还需运行：

```sh
nix develop .#gui -c cargo fmt --all -- --check
nix develop .#gui -c cargo build --all-targets --features gui-tests --locked
nix develop .#gui -c cargo test --features gui-tests --bin basrw --locked
nix develop .#gui -c cargo clippy --all-targets --features gui-tests --locked -- -D warnings
```

CI 分别执行默认测试与 headless GUI 测试，不读取 `.env`，也不调用真实模型。
状态回归通过受控事件顺序和 ACP fixture 验证：设置响应与通知的提交顺序、
启动期间历史淘汰、快照版本合并、发送回执与事件交错以及队列编辑保序。
这些测试不使用固定等待时间推测操作是否完成。

## 自动系统测试

宿主机需要 Linux 和启用了 flakes 的 Nix，当前提供 `x86_64-linux`、
`aarch64-linux` 两种配置。QEMU、客户机系统和测试工具由 Nix 提供。

```sh
./scripts/test-system check
```

脚本包含未提交、尚未被 Git 跟踪的源码；不会修改 Git 暂存区，也不会把
`target/`、`.git/` 或测试输出复制到源码快照中。

第一次执行会下载依赖并构建 BASR 和虚拟机。后续执行复用 Nix 缓存。
KVM 可用时使用硬件加速，否则回退到软件模拟；客户机配置为 2 核、1536 MiB 内存，
自动测试的运行超时为 15 分钟，不包含构建时间。

测试脚本检查：

- 普通用户运行的 `basrd` 通过 `Type=notify` 就绪，并持续发送 watchdog 通知。
- journald 收到服务日志。
- CLI 创建、提示、改名、历史分页和删除会话。
- 真实的用户 D-Bus 将通知送到记录通知内容的测试服务。
- 受控 ACP 后端的消息、取消和子进程回收。
- 真实 pi 程序的安装、ACP 启动及缺少认证时的失败行为。
- systemd 正常停止服务时关闭订阅，退出进程并释放监听端口。
- 必需的通知服务缺失时启动失败，`auto` 策略允许无桌面启动。
- 服务重启后从空的内存会话集合开始运行。

这一入口不注入密钥、不调用外部模型；生成行为使用 echo 和受控 ACP 后端。
真实 pi 的模型调用由下面的 `pi` 入口验证。

成功后的日志和 JUnit 报告位于：

```text
.test-results/system/
```

该目录是指向 Nix 测试产物的符号链接。产物包含 `junit.xml`，以及客户机导出的
系统日志、有效服务单元、通知记录和订阅输出；部分诊断文件只有对应步骤执行后才会存在。

如果测试失败，先看终端输出中的失败子测试和 Nix 提示的日志命令。
入口默认使用 `--keep-failed`，Nix 会保留失败构建目录并打印位置，便于查找诊断文件。
失败时不要把 `.test-results/system/` 中可能保留的上一次成功报告当作本次结果。

源码没有变化时，成功的测试结果也会被缓存。需要强制重新执行时：

```sh
./scripts/test-system check --rebuild
```

## 虚拟机 shell

```sh
./scripts/test-system shell
```

这会启动虚拟机、通知服务和 `basrd`，然后直接进入客户机的 root shell。
守护进程本身仍以 `basr` 用户运行。可以直接执行：

```sh
basrctl agents
basrctl session create --cwd /home/basr --title manual
basrctl session create --agent pi --cwd /home/basr --title pi-manual
systemctl status basrd --no-pager
journalctl -u basrd --no-pager -n 100
```

终端通过测试框架的 SSH/VSOCK 通道连接客户机，支持交互程序和 Ctrl-C，
无需配置 SSH 密钥或宿主机端口。输入 `exit` 或按 Ctrl-D
会结束会话并释放 QEMU 进程。每次启动使用新的 `.test-results/shell.XXXXXX/`
日志目录，入口会打印具体位置。`driver.xml` 记录驱动日志，`console.log` 在退出时
保存客户机控制台输出。虚拟磁盘和通信套接字放在独立的临时目录，退出时
自动清理，因此它是一次性环境，不用于保存工作文件，也不会与另一次调试共用状态。

## 真实 pi 与 DeepSeek

虚拟机安装真实的 pi 0.75.4（由 `flake.lock` 固定）和 pi-acp 0.0.32
（由 `nix/pi-acp/package-lock.json` 与 Nix 哈希固定），BASR 将其注册为 `pi` 后端。
使用已有的 [pi-acp 适配器](https://github.com/svkozak/pi-acp) 连接 pi 的 RPC 模式。
适配器 0.0.33 要求较新的 pi，本环境使用与当前锁定的 pi 配套的 0.0.32。

默认模型为 `deepseek-flash`，通过 DeepSeek 官方的 OpenAI 兼容接口调用。
基础地址与模型名称见 [DeepSeek API 文档](https://api-docs.deepseek.com/)。
测试关闭 thinking 与自动重试，并将每次回复上限设为 2048 tokens。

在仓库根目录准备 `.env`（首次配置时从示例复制）：

```sh
cp .env.example .env
chmod 600 .env
```

填入自己的密钥：

```dotenv
DEEPSEEK_API_KEY=your-api-key
```

宿主机环境中的非空 `DEEPSEEK_API_KEY` 优先于 `.env`，可用于 CI。
驱动使用 `python-dotenv` 解析文件，不执行 shell 命令，也不展开变量引用。
`.env` 与 `.env.*`（除 `.env.example`）均被 Git 忽略，并排除在测试脚本的 Nix
源码快照之外。驱动在运行时生成临时的 pi `auth.json`，复制到虚拟机后删除宿主机
临时文件；客户机里的认证文件归 `basr` 用户所有，权限为 `0600`。
模型设置保存在 `tests/system/pi/settings.json` 和 `models.json`。

准备好认证后运行：

```sh
./scripts/test-system pi
```

这个入口每次都会实际调用模型并产生 API 用量，复用的是环境构建缓存。
它通过 `basrctl` 创建 pi 会话，让 DeepSeek 指挥 pi 读取随机内容的文件、写出
大写转换结果并读回验证；同时检查 ACP 工具事件、回合完成和会话删除后的进程回收。
它在 Nix 构建沙箱外启动 QEMU，因此客户机能够联网，密钥不进入 Nix store。

日志写入本次运行打印的 `.test-results/pi.XXXXXX/`，包括 `driver.xml`、
`pi-history.log`、`output.txt`、`pi-daemon.log` 和 `console.log`。
失败时以本次日志为准；部分文件只有对应步骤完成后才存在。

`shell` 入口也会读取同一份 `.env`。在客户机里可以操作真实 pi：

```sh
basrctl session create --agent pi --cwd /home/basr --title manual
basrctl prompt <session-id> '在当前目录创建 hello.txt，内容为 hello from pi'
basrctl attach <session-id> --cursor 0
```

pi 的文件与 shell 工具由 pi 自己执行，当前适配器不使用 BASR 的工具权限回调。
在这个环境中它以 `basr` 用户运行，受 `basrd` 的 systemd 文件写入限制约束。
`PI_OFFLINE=1` 只关闭 pi 的启动更新检查等网络活动，模型请求仍正常联网。

## Python 测试驱动

开发自动化测试或同时控制多台虚拟机时使用：

```sh
./scripts/test-system driver
```

这个入口才会显示 Python／CPython 控制台。

在控制台中，可以运行完整测试并在结束后继续检查客户机：

```python
run_tests()
print(machine.succeed("journalctl -u basrd --no-pager -n 100"))
```

完整测试结束时会停止 `basrd`，并留下测试无桌面启动所用的临时 drop-in。
需要手动操作一个刚启动的环境时，在新的交互控制台执行：

```python
start_all()
machine.wait_for_unit("basr-test-notifications.service", user="basr")
machine.succeed("systemctl start basrd")
print(machine.succeed("runuser -u basr -- basrctl agents"))
print(machine.succeed(
    "runuser -u basr -- basrctl session create --cwd /home/basr --title manual"
))
```

后续把输出中的会话 id 传给 `basrctl prompt`、`history` 等命令即可。
退出 Python 控制台时测试驱动会关闭虚拟机。交互目录保留日志供排查。

只想预先构建环境、不启动虚拟机时：

```sh
./scripts/test-system build
```

## 扩展测试

| 文件 | 职责 |
| --- | --- |
| `tests/system/test.py` | 黑箱测试步骤，使用测试驱动的 `machine` 与 `subtest` |
| `tests/system/shell.py` | 启动服务并进入客户机命令 shell |
| `nix/tests/shell.nix` | 交互终端所用的 SSH/VSOCK 与驱动配置 |
| `nix/tests/system.nix` | 客户机用户、服务、软件包、内存和超时配置 |
| `nix/pi-acp/` | 固定版本的 pi-acp 打包及 npm 依赖锁 |
| `nix/tests/pi.nix`、`tests/system/pi.py` | 真实 pi 与模型的端到端测试 |
| `tests/system/pi_config.py` | 将宿主机私有配置注入客户机 |
| `tests/system/pi/` | 不含密钥的 DeepSeek 模型及默认设置 |
| `tests/system/notifications.py` | 在真实 D-Bus 上记录通知的测试服务 |
| `src/bin/basrd/agent/acp/assets/fixture.sh` | Rust 进程测试与虚拟机共用的受控 ACP 后端 |
| `nix/package.nix` | 从锁定依赖构建并测试 Rust 包 |

新增场景优先增加独立的 `with subtest("场景名称"):` 块。通过 `wait_for_unit`、
`wait_for_file`、`wait_until_succeeds` 等等待可观察状态，避免固定等待时间。
需要额外系统依赖时，把它加入客户机配置，测试过程不临时下载软件。

测试驱动接口见 [NixOS 官方测试文档](https://nixos.org/manual/nixos/stable/#sec-nixos-tests)。
