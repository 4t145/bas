# 安装与后端配置

## Linux / NixOS 安装

在仓库根目录执行，Nix 需启用 flakes：

```sh
./scripts/install-basrd --service
./scripts/install-basrw
basrctl agents
basrw
```

`install-basrd` 只安装 release 版 `basrd` 和 `basrctl`，不安装、更新或卸载任何外部
ACP 适配器及其 CLI，也不创建或修改用户配置。外部工具由用户按各自官方文档独立安装和管理。
未配置后端时只提供 `embedded` echo 后端；安装外部工具后再手动添加对应命令。
`--service` 链接并启用 systemd **用户服务**，立即启动；无需 root，服务使用当前用户的目录与登录状态。
脚本包含尚未加入 Git 的源码，排除 `.env`、测试输出和构建目录。

仅安装守护进程、不启用服务：

```sh
./scripts/install-basrd
basrd
```

默认监听 `127.0.0.1:7337`。GUI 和 CLI 默认连接这个地址。
无需 GUI 时，只安装 basrd 即可，守护进程包不引入图形运行库。

源码均已加入 Git 后，也可使用 `nix profile install .#basrd` 或 `nix run .#basrd`。
NixOS / Home Manager 可将 `inputs.basr.packages.${pkgs.stdenv.hostPlatform.system}.basrd`
加入 `environment.systemPackages` / `home.packages`，服务另行声明。
BASR flake 不再提供外部适配器安装包；在 NixOS 上请自行选择适合系统的上游发行方式。

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

`ListAgents` 只返回当前可找到可执行文件的工具条目；缺失、目录或无执行权限的 ACP 程序会被隐藏，
但配置文件仍保留，安装工具或修正路径后下次刷新即可重新出现。
查找与实际启动共用 daemon 的 PATH（含该 Agent 的 `env.PATH` 覆盖），不使用 GUI 的 PATH。
设置页的显式连接检测会验证 ACP 握手；“已找到程序”与“已连通”是两个状态。
使用此检测功能需要同时升级 `basrw` 与 `basrd`，旧版 daemon 会提示不支持。

修改 Agent 目录不需要重启。升级 daemon 可执行文件仍需重启；会话和回放窗口保存在内存中，
重启会停止当前会话。不要以重启作为设置保存动作。
`./scripts/dev-local` 仍显式使用开发用的 pi 目录；日常使用配置文件时启动上述用户服务。

## Codex

按 [Codex ACP 官方安装说明](https://github.com/agentclientprotocol/codex-acp#installation)
独立安装 `@agentclientprotocol/codex-acp`。例如在满足上游运行环境要求后执行：

```sh
npm install -g @agentclientprotocol/codex-acp
command -v codex-acp
codex-acp cli login
```

上游适配器自身附带兼容的 Codex CLI；其版本、升级和认证由上游工具及用户管理，
BASR 不锁定、注入或覆盖 CLI 路径。需要自定义 CLI 时遵循上游的 `CODEX_PATH` 说明。
已有 Codex 登录可继续使用；登录命令应在运行 basrd 的同一用户下执行。

把 `command -v codex-acp` 得到的、独立安装的可执行文件绝对路径填入
`command.program`，`command.args` 为 `[]`；确认 daemon 的 PATH 能找到它时也可写 `codex-acp`。
全局 npm 的安装目录不一定在 systemd 用户服务 PATH 内；不要把 shell 中能启动等同于服务能启动。
保存后可执行 `basrctl session create --agent codex --cwd "$PWD"`，
或在 GUI 的 Agent 选择器中选 `codex`。模型等配置选项与 `/` 命令由后端提供。
模型与认证继续使用现有 Codex 配置，basrd 不生成另一套凭据或修改 Codex 权限设置。
适配器实现见 [codex-acp](https://github.com/agentclientprotocol/codex-acp)，
底层接口见 [Codex App Server 文档](https://learn.chatgpt.com/docs/app-server)。

### 从旧版捆绑安装迁移

`--with-codex` 已移除，使用它会在任何安装动作前报错并指向本文；改用
`./scripts/install-basrd --service`。更新 BASR 不会卸载旧适配器，也不会修改已有配置。
以前安装的 `~/.nix-profile/bin/codex-acp` 可能仍然存在，已有配置也可能继续指向它。
请按官方文档独立安装适配器，检查 `command -v codex-acp` 的结果和服务实际使用的路径，
再自行把后端命令改为所选安装路径。旧 profile 条目由用户决定是否保留；
本次迁移不自动删除任何包、凭据或配置。已有 `CODEX_PATH` / `env` 同样保留，请自行确认是否仍需要。

## OpenCode 与 Pi

按 [OpenCode 官方安装说明](https://opencode.ai/docs/) 独立安装 OpenCode；
它原生支持 ACP，使用 `opencode acp`，见[官方 ACP 文档](https://opencode.ai/docs/acp/)。
配置程序为 `opencode`（或其绝对路径），参数为 `["acp"]`。
Pi 的原生接口为 `pi --mode rpc`，不是 ACP；使用独立
[pi-acp adapter 官方安装说明](https://github.com/svkozak/pi-acp#prerequisites)
安装 Pi 本体及适配器，并确保二者在服务环境中可用。例如：

```sh
npm install -g @earendil-works/pi-coding-agent pi-acp
pi-acp --terminal-login
command -v pi-acp
```

配置程序为 `pi-acp`（或其绝对路径），参数为 `[]`，不要把登录参数放入日常 ACP 启动配置。
仓库的 Pi 固定依赖仅供开发 shell 和虚拟机测试，不属于 BASR 安装包或用户安装渠道。
配置命令不执行安装、下载、登录或模型请求。
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

更新本地安装时重新运行安装脚本即可：脚本会替换同名的旧 profile 条目，`--service` 还会重启
用户服务，让新可执行文件立即生效；后端配置保留。

```sh
./scripts/install-basrd --service
./scripts/install-basrw
```

重启会停止当前会话，会话与回放窗口只保存在内存中；只换 GUI 时不需要重启服务。
卸载 BASR 执行 `nix profile remove basrd basrw`（未安装的条目可省略，实际名称用
`nix profile list` 查看），再 `systemctl --user disable basrd`。GUI 的安装、更新与卸载见
[GUI 说明](gui.md)。外部工具由用户单独管理，不随 BASR 卸载。
