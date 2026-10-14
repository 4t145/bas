# 测试环境

日常代码测试运行在开发环境中；涉及服务管理、进程退出和桌面通知的测试运行在
QEMU 的 NixOS 虚拟机中。Nix 依赖由 `flake.lock` 固定，Rust 依赖由 `Cargo.lock` 固定。

## 日常测试

安装脚本的离线回归测试使用临时目录和模拟 Nix，不安装包或操作真实服务：

```sh
python3 tests/install_bas.py -v
```

真实 Nix source 回归验证 metadata 的锁定 URL round-trip 和源码过滤，
覆盖 Git 子模块与 path 快照的类型边界；使用临时目录，不修改系统 profile：

```sh
python3 tests/nix_source.py -v
```

本机 GUI 开发见 [本机启动](gui.md#本机启动)。外部 Agent 通过 ACP Registry
按需安装；以下自动测试使用 embedded echo 和受控 ACP fixture，不读取凭据或调用真实模型。

在物理 checkout 先执行 `git submodule update --init --recursive`，然后在仓库根目录运行：

```sh
nix develop -c cargo test --all-targets
```

已经进入开发 shell 时可以直接运行 `cargo test --all-targets`。这一层包括
Rust 单元测试、gRPC 和进程集成测试，不启动虚拟机。

默认目标不包含可选的 GUI。完整本地验收还需运行：

```sh
nix develop .#gui -c cargo fmt --all -- --check
nix develop .#gui -c cargo build --all-targets --features gui-tests --locked
nix develop .#gui -c cargo test --features gui-tests --bin basg --locked
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

第一次执行会下载依赖并构建 Bas 和虚拟机。后续执行复用 Nix 缓存。
KVM 可用时使用硬件加速，否则回退到软件模拟；客户机配置为 2 核、1536 MiB 内存，
自动测试的运行超时为 15 分钟，不包含构建时间。

测试脚本检查：

- 普通用户运行的 `bas` 通过 `Type=notify` 就绪，并持续发送 watchdog 通知。
- journald 收到服务日志。
- CLI 创建、提示、改名、历史分页和删除会话。
- 真实的用户 D-Bus 将通知送到记录通知内容的测试服务。
- 受控 ACP 后端的消息、取消和子进程回收。
- systemd 正常停止服务时关闭订阅，退出进程并释放监听端口。
- 必需的通知服务缺失时启动失败，`auto` 策略允许无桌面启动。
- 测试环境显式使用 `BAS_STORAGE=memory`，服务重启后从空的内存会话集合开始运行。

这些入口不预装外部 Agent，也不读取仓库 `.env`。产品默认的 `localdb` 存储不受影响。

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

这会启动虚拟机、通知服务和 `bas`，然后直接进入客户机的 root shell。
守护进程本身仍以 `bas` 用户运行。可以直接执行：

```sh
bast agents
bast session create --agent embedded --cwd /home/bas --title manual
bast session create --agent normal --cwd /home/bas --title acp-manual
systemctl status bas --no-pager
journalctl -u bas --no-pager -n 100
```

终端通过测试框架的 SSH/VSOCK 通道连接客户机，支持交互程序和 Ctrl-C，
无需配置 SSH 密钥或宿主机端口。输入 `exit` 或按 Ctrl-D
会结束会话并释放 QEMU 进程。每次启动使用新的 `.test-results/shell.XXXXXX/`
日志目录，入口会打印具体位置。`driver.xml` 记录驱动日志，`console.log` 在退出时
保存客户机控制台输出。虚拟磁盘和通信套接字放在独立的临时目录，退出时
自动清理，因此它是一次性环境，不用于保存工作文件，也不会与另一次调试共用状态。

## Python 测试驱动

开发自动化测试或同时控制多台虚拟机时使用：

```sh
./scripts/test-system driver
```

这个入口才会显示 Python／CPython 控制台。

在控制台中，可以运行完整测试并在结束后继续检查客户机：

```python
run_tests()
print(machine.succeed("journalctl -u bas --no-pager -n 100"))
```

完整测试结束时会停止 `bas`，并留下测试无桌面启动所用的临时 drop-in。
需要手动操作一个刚启动的环境时，在新的交互控制台执行：

```python
start_all()
machine.wait_for_unit("bas-test-notifications.service", user="bas")
machine.succeed("systemctl start bas")
print(machine.succeed("runuser -u bas -- bast agents"))
print(machine.succeed(
    "runuser -u bas -- bast session create --cwd /home/bas --title manual"
))
```

后续把输出中的会话 id 传给 `bast prompt`、`history` 等命令即可。
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
| `tests/system/notifications.py` | 在真实 D-Bus 上记录通知的测试服务 |
| `src/bin/bas/agent/acp/assets/fixture.sh` | Rust 进程测试与虚拟机共用的受控 ACP 后端 |
| `nix/package.nix` | 从锁定依赖构建并测试 Rust 包 |

新增场景优先增加独立的 `with subtest("场景名称"):` 块。通过 `wait_for_unit`、
`wait_for_file`、`wait_until_succeeds` 等等待可观察状态，避免固定等待时间。
需要额外系统依赖时，把它加入客户机配置，测试过程不临时下载软件。

测试驱动接口见 [NixOS 官方测试文档](https://nixos.org/manual/nixos/stable/#sec-nixos-tests)。
