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
Codex ACP 适配器及兼容的 Codex CLI，全新配置会创建含 `embedded`、`codex` 的 `settings.json`。
已有统一配置或旧版配置不会被覆盖，可在设置窗口合并 Codex 条目并完成迁移。
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

GUI 与 daemon 默认读取 `$XDG_CONFIG_HOME/basr/settings.json`，未设置 XDG 时为
`~/.config/basr/settings.json`。两者都支持 `--settings-file` / `BASR_SETTINGS_FILE`；
使用不同路径的服务不会看到另一路径中的修改。

```json
{
  "appearance": {},
  "daemon": {
    "agents": [
      { "id": "embedded", "kind": "embedded" },
      {
        "id": "codex",
        "kind": "acp",
        "command": { "program": "codex-acp", "args": [] }
      },
      {
        "id": "opencode",
        "kind": "acp",
        "command": { "program": "opencode", "args": ["acp"] }
      },
      {
        "id": "pi",
        "kind": "acp",
        "command": { "program": "pi-acp", "args": [] }
      }
    ]
  }
}
```

`id` 是 GUI 中显示的后端名，必须唯一；`embedded` 是本机 echo 测试后端。
添加其他 ACP 后端时，使用 `kind: "acp"`，把启动程序与参数分别放在 `command.program`、
`command.args` 中。程序可用绝对路径；参数数组保留空格边界，不经过 shell 解析，也不会展开 `~` 或 `$HOME`。
`env` 可为该后端覆盖非秘密环境变量；其余变量继承自 basrd。凭据使用各 Agent 的原生
登录或服务环境，不应把 API key 填入共享 JSON。设置页保留已有 `env`，但不收集密钥。

ACP 后端可设置 `cancellation_grace_ms`，表示发送取消后等待本轮结束的宽限期，
单位为毫秒，必须大于零，默认 `5000`。例如需要较长工具清理时间的后端可以设置
`"cancellation_grace_ms": 15000`。超过期限仍会判定后端不可继续并回收进程；
该选项不会关闭强制清理机制。

选择目录的优先级为：`--agents` / `BASRD_AGENTS` 简写目录、显式
`--agents-file` / `BASRD_AGENTS_FILE` 旧格式数组文件、统一 settings。
同一个参数的命令行值优先于环境变量。
显式指定的文件不存在、JSON 无效、重复 ID 或 ACP 缺少启动程序都会使启动失败；
默认统一文件不存在时只读导入同目录旧 `appearance.json` 和 `agents.json`；都不存在才使用
内置后端。读取不会迁移或删除文件；设置窗口第一次成功保存才生成统一文件，并保留另一 section。

文件配置每秒自动检查，整份目录校验成功后原子替换；非法更新或已读取文件消失时保留上次有效目录。
新建会话使用新配置，运行中的会话及后台进程不变。`--agents` 字符串覆盖不监听文件。
GUI 的“Agent 服务”仅编辑本机配置；保存成功不等于已经确认某个远端或旧版本 daemon 采用该配置。

修改 Agent 目录不需要重启。升级 daemon 可执行文件仍需重启；会话和回放窗口保存在内存中，
重启会停止当前会话。不要以重启作为设置保存动作。
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

## OpenCode 与 Pi

OpenCode 原生支持 ACP，使用 `opencode acp`，见[官方 ACP 文档](https://opencode.ai/docs/acp/)。
Pi 的原生接口为 `pi --mode rpc`，不是 ACP；使用独立
[pi-acp adapter](https://github.com/svkozak/pi-acp)，并确保其需要的 Pi 本体在服务环境中可用。
本仓库提供 Nix `pi-acp` 包。预设只是命令模板，不执行安装、下载、登录或模型请求。
如果服务找不到程序，可填写绝对路径；GUI 自身的 PATH 不代表 systemd 服务的 PATH。

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
其他位置的 ACP 程序应在 `settings.json` 的 `daemon.agents` 中使用绝对路径。此运行时环境文件不进入 Nix 安装包。

更新本地安装时，先停止并禁用服务（同时移除旧服务链接），再替换 profile 条目；后端配置保留：

```sh
systemctl --user disable --now basrd
nix profile remove basrd codex-acp
./scripts/install-basrd --with-codex --service
```

如果只安装了 basrd，省略未安装的 `codex-acp` 条目；实际名称可用 `nix profile list` 查看。
卸载时执行前两条即可。GUI 的安装、更新与卸载见 [GUI 说明](gui.md)。
