# 测试环境

日常代码测试运行在开发环境中。Nix 依赖由 `flake.lock` 固定，Rust 依赖由
`Cargo.lock` 固定。

## 日常测试

安装脚本的离线回归测试使用临时目录和模拟 Nix，不安装包或操作真实服务：

```sh
python3 tests/install_bas.py -v
python3 tests/install_basg_dev.py -v
```

真实 Nix source 回归验证 metadata 的锁定 URL round-trip 和源码过滤，
覆盖不含 vendor 源码的 package source 和 Git 宏依赖锁定；使用临时目录，不修改系统 profile：

```sh
python3 tests/nix_source.py -v
```

本机 GUI 开发见 [本机启动](gui.md#本机启动)。外部 Agent 通过 ACP Registry
按需安装；以下自动测试使用 embedded echo 和受控 ACP fixture，不读取凭据或调用真实模型。

无需初始化子模块，在仓库根目录运行：

```sh
nix develop -c cargo test --all-targets
```

已经进入开发 shell 时可以直接运行 `cargo test --all-targets`。这一层包括
Rust 单元测试、gRPC 和进程集成测试。

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

## 系统测试

QEMU/NixOS 系统测试目前不在仓库中，后续统一补全。当前的系统级覆盖由
`cargo test --all-targets` 中的进程集成测试和真实 gRPC 测试承担。
