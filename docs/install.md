# 安装与后端配置

## Linux / NixOS 安装

在仓库根目录执行，Nix 需启用 flakes：

```sh
./scripts/install-basrd --with-codex --service
./scripts/install-basrw
basrctl agents
basrw
```

`install-basrd` 安装 release 版 `basrd` 和 `basrctl`；`--with-codex` 同时安装固定版本的
Codex ACP 适配器及兼容的 Codex CLI，在用户后端配置不存在时创建 `embedded`、`codex` 两项。
已有配置不会被覆盖，需要自行把 Codex 条目合并进去。
`--service` 链接并启用 systemd **用户服务**，立即启动；无需 root，服务使用当前用户的目录与登录状态。
脚本包含尚未加入 Git 的源码，排除 `.env`、测试输出和构建目录。

仅安装守护进程、不启用服务或 Codex：

```sh
./scripts/install-basrd
basrd
```

默认监听 `127.0.0.1:7337`。GUI 和 CLI 默认连接这个地址。
无需 GUI 时，只安装 basrd 即可，守护进程包不引入图形运行库。

源码均已加入 Git 后，也可使用 `nix profile install .#basrd .#codex-acp` 或 `nix run .#basrd`。
NixOS / Home Manager 可将 `inputs.basr.packages.${pkgs.stdenv.hostPlatform.system}.basrd`
及 `.codex-acp` 加入 `environment.systemPackages` / `home.packages`，服务另行声明。

## 配置 ACP 后端

默认读取 `$XDG_CONFIG_HOME/basr/agents.json`，未设置 XDG 时为 `~/.config/basr/agents.json`。
可以设置 `BASRD_AGENTS_FILE`，或使用 `basrd --agents-file /path/to/agents.json` 指定另一份目录。

```json
[
  { "id": "embedded", "kind": "embedded" },
  {
    "id": "codex",
    "kind": "acp",
    "command": { "program": "codex-acp", "args": [] },
    "env": {}
  }
]
```

`id` 是 GUI 中显示的后端名，必须唯一；`embedded` 是本机 echo 测试后端。
添加其他 ACP 后端时，使用 `kind: "acp"`，把启动程序与参数分别放在 `command.program`、
`command.args` 中。程序可用绝对路径；参数数组保留空格边界，不经过 shell 解析，也不会展开 `~` 或 `$HOME`。
`env` 可为该后端覆盖环境变量；其余变量继承自 basrd。

ACP 后端可设置 `cancellation_grace_ms`，表示发送取消后等待本轮结束的宽限期，
单位为毫秒，必须大于零，默认 `5000`。例如需要较长工具清理时间的后端可以设置
`"cancellation_grace_ms": 15000`。超过期限仍会判定后端不可继续并回收进程；
该选项不会关闭强制清理机制。

选择目录的优先级为：`--agents` / `BASRD_AGENTS` 简写目录、显式 JSON 文件、默认用户文件、
内置 `embedded`。同一个参数的命令行值优先于环境变量。
显式指定的文件不存在、JSON 无效、重复 ID 或 ACP 缺少启动程序都会使启动失败；
默认文件不存在时才使用内置后端。

后端目录在 basrd 启动时读取。修改后重启服务，再刷新 GUI：

```sh
systemctl --user restart basrd
basrctl agents
```

basrd 的会话和回放窗口保存在内存中，重启会停止当前会话。
`./scripts/dev-local` 仍显式使用开发用的 pi 目录；日常使用配置文件时启动上述用户服务。

## Codex

适配器使用 `@agentclientprotocol/codex-acp`，通过 Codex App Server 对接，并附带匹配的 Codex CLI。
本仓库锁定适配器 1.12.0、CLI 0.154.0，无需依赖 Zed 的缓存或全局 npm 安装。
已有 Codex 登录可直接使用；新机器在当前用户下登录：

```sh
codex-acp cli login
basrctl session create --agent codex --cwd "$PWD"
```

也可以在 GUI 的 Agent 选择器中选 `codex`。会话创建后，模型等配置选项与 `/` 命令由后端提供。
模型与认证继续使用现有 Codex 配置，basrd 不生成另一套凭据或修改 Codex 权限设置。
适配器实现见 [codex-acp](https://github.com/agentclientprotocol/codex-acp)，
底层接口见 [Codex App Server 文档](https://learn.chatgpt.com/docs/app-server)。

## 服务管理

```sh
systemctl --user status basrd
journalctl --user -u basrd -f
systemctl --user stop basrd
systemctl --user start basrd
```

用户服务默认随用户会话启动，支持就绪通知、watchdog 与异常退出后重启。
可在 `~/.config/basr/basrd.env`（或 `$XDG_CONFIG_HOME/basr/basrd.env`）中设置服务环境变量，例如：

```ini
BASRD_LISTEN=127.0.0.1:7337
BASRD_HISTORY=8192
```

服务的 PATH 包含 `~/.local/bin`、`~/.nix-profile/bin` 和系统软件路径；
其他位置的 ACP 程序应在 `agents.json` 中使用绝对路径。此运行时环境文件不进入 Nix 安装包。

更新本地安装时，先停止并禁用服务（同时移除旧服务链接），再替换 profile 条目；后端配置保留：

```sh
systemctl --user disable --now basrd
nix profile remove basrd codex-acp
./scripts/install-basrd --with-codex --service
```

如果只安装了 basrd，省略未安装的 `codex-acp` 条目；实际名称可用 `nix profile list` 查看。
卸载时执行前两条即可。GUI 的安装、更新与卸载见 [GUI 说明](gui.md)。
