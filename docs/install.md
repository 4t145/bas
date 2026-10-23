# 安装与后端配置

## Linux / NixOS 安装

在仓库根目录执行，Nix 需启用 flakes：

```sh
make install-bas-service
make install-basg
bast agents
basg
```

### 快速试用界面改动

```sh
make install-basg-dev
basg-dev
```

此入口在 Nix GUI 开发环境内运行 Cargo **dev profile** 增量编译，复用当前 checkout
的 `target/`（或已有的 `CARGO_TARGET_DIR`），不清理缓存、不进行沙箱内 release 重编译。
首次构建、依赖／工具链／编译选项变化仍可能较慢。
Nix 只将已编译程序、桌面入口和图形运行库封装为不可变包，再安装到当前 profile。
因此删除构建目录或移动源码后，已安装程序仍可启动，运行库也由 profile 保持存活。

开发版命令为 **`basg-dev`**，桌面入口为 **BasG（开发版）**；
正式版的 `basg` 命令和入口保持不变。二者连接同一个 daemon、使用相同配置，
不要把重新打开正式版误认为测试了开发版。更新后关闭旧开发进程，再打开开发版。
开发安装不修改配置、不安装／重启后端，也不代替测试；安装前运行相关检查。
涉及 daemon 或 RPC 的变更仍需使用 `make install-bas-service` 更新后端。

可用 `NIX_PROFILE_OPTIONS=--offline` 禁止 Nix 联网，或用 `--profile /path` 指定 profile。
卸载开发版使用 `nix profile remove basg-dev`，不会卸载正式版。
正式发布继续使用下述 Nix release 安装流程。

### 安装独立 ACP 测试 Agent

```sh
make install-acp-test-agent
acp-test-agent --help
```

此目标只安装 `acp-test-agent`，使用它自己的锁文件和 Rust 工具链，在 Nix 构建中执行其测试，
不构建 BasG、不安装服务，也不修改 Agent 配置。它不包含在 `make install` 的默认安装集合中。
同样支持 `NIX_PROFILE_OPTIONS`；卸载使用 `nix profile remove acp-test-agent`。

安装后，在 BasG 的“添加自定义智能体”中填写：

- ID：`acp-test-agent`
- 程序：安装输出中的 `/nix/store/.../bin/acp-test-agent` 绝对路径
- 参数（JSON）：`["--protocol", "v1"]`

也可用 `readlink -f "$(command -v acp-test-agent)"` 查询程序路径。固定 store 路径不依赖
已运行 daemon 的 PATH；升级后应更新自定义 Agent 的程序路径。
测试 Agent 由客户端按需以 stdio 启动，不应当单独作为常驻 systemd 服务运行。

新建会话后发送 `/help`。建议先用 `/rich` 测试图片／音频等内容、`/tool` 测试工具卡、
`/plan` 测试计划、`/permission` 和 `/elicit` 测试交互、`/sleep 10000` 测试取消。
它目前没有 subagent 演示，不能用来验证子 Agent 角标；`/terminal` 会说明当前客户端
未声明终端执行能力。命令与限制见 [测试 Agent 文档](../acp-test-agent/README.md)。
`/rich` 的现有音频样本是 0 帧 WAV，占位内容可用于检查协议传输和音频卡片，
不能用来验收实际发声；播放器听音验收需要另行提供含有效采样的音频。

NixOS 声明式安装可添加到 `environment.systemPackages`：

```nix
inputs.bas.packages.${pkgs.stdenv.hostPlatform.system}.acp-test-agent
```

Home Manager 则放入 `home.packages`。Flake 同时提供 `#acp-test-agent` package、app 和 check，
支持 `x86_64-linux`、`aarch64-linux`；测试工具包不依赖主项目的 GUI。

`make install-bas` 只安装 release 版 `bas` 和 `bast`，不安装、更新或卸载任何外部
ACP 适配器及其 CLI，也不创建或修改用户配置。外部后端可通过运行中的 daemon 按需从 ACP Registry 安装，或由用户按官方文档独立管理。
未配置后端时只提供 `embedded` echo 后端。
`make install-bas-service` 还会链接并启用 systemd **用户服务**，立即启动；无需 root，服务使用当前用户的目录与登录状态。
优先使用 Makefile 的快照安装路径。它复用 `scripts/install-bas` 与 `scripts/install-basg`，
包含尚未加入 Git 的第一方源码，排除 `.git`、`.delta`、`.env` 等
元数据、测试输出和构建目录。不需要初始化子模块；`gds-uim` 宏由 Cargo.lock 与
Nix 固定输出 hash 获取。path 快照不附加 Git fetching 参数。

`make install` 同时安装 daemon、CLI 和 GUI，不启用用户服务；即使使用 `make -j`，
安装目标也会顺序执行，避免同时修改同一个 Nix profile。`make help` 列出所有目标。
额外的 `nix profile add` 参数通过 `NIX_PROFILE_OPTIONS` 传递，例如：

```sh
make install NIX_PROFILE_OPTIONS='--profile /path/to/profile -L'
```

仅安装守护进程、不启用服务：

```sh
make install-bas
bas
```

默认使用同用户本地 IPC，GUI 和 CLI 默认连接 `local`，无需配对。
TCP 仍可显式开启，连接格式和旧版迁移见 [服务连接类型](connections.md)。
无需 GUI 时，只安装 bas 即可，守护进程包不引入图形运行库。

源码均已加入 Git 后，也可直接使用本地 flake：

```sh
nix profile add .#bas
nix run .#bas
nix run .#bast -- agents
```

`bas` 包同时包含 daemon 和 CLI。
NixOS / Home Manager 可将 `inputs.bas.packages.${pkgs.stdenv.hostPlatform.system}.bas`
加入 `environment.systemPackages` / `home.packages`，服务另行声明。
远端输入可使用标准 GitHub fetcher：

```nix
inputs.bas.url = "github:4t145/bas";
```

若旧版已将带 `submodules=1` 的 path URL 写入 profile，更正源码不会自动修复已有坏记录；
问题背景见 [Nix source 类型边界](upstream-ui.md#nix-source-类型边界)。
Bas flake 本身不捆绑外部适配器。Nix 安装的 daemon 为托管 ELF 程序提供安装时的解释器和运行库修补；
用户独立管理的程序继续使用自己的发行方式。

### 已写入坏 path URL 的 profile

若 `nix profile list` 已报 `path URL ... has unsupported parameter 'submodules'`，
先更新到修复后的源码，再核对当前 generation 与之前正常 generation 的 `manifest.json`。
不要直接修改 `/nix/store`，也不要删除整个 profile。

只有在确认两代相比**仅多了此次 Bas 安装，其他条目的 storePaths、优先级和来源完全一致**
时，才可回到那一代并重新安装。将下面的 `N` 替换为核对过的正常代数：

```sh
nix profile rollback --dry-run --to N
nix profile rollback --to N
make install-bas
```

若问题来自 BasG，最后一步使用 `make install-basg`。使用自定义 profile 时，
上述 Nix 命令都要指定相同的 `--profile`，Make 通过 `NIX_PROFILE_OPTIONS` 传递。
找不到满足条件的正常代时，不要盲目回滚；应保留清单并单独恢复。
旧的坏 generation 会继续保留，`nix profile history` 因为读取它仍可能报错；
当前 profile 修好后无需为消除历史显示错误而自动删除回滚记录。

## 从 ACP Registry 安装后端

在 GUI 的 Agent 选择器中点“安装 Agent…”，或打开设置的 Agent 服务页。
搜索后端并点击“安装”；daemon 下载对应平台的二进制，或安装固定版本的 npm 包，
自动写入启动配置并检测 ACP 连接。需要认证时选择 Agent 提供的登录方式，完成后自动重新检查。
模型、账号和工具权限继续使用后端自己的配置。

无 GUI 时使用同一套 daemon 接口：

```sh
bast agent catalog
bast agent catalog --refresh
bast agent install codex-acp
bast agent login codex-acp
bast agent install opencode
bast agent install pi-acp
bast agent check pi-acp
```

多个认证方式可用时，`agent login` 列出方法 ID，使用 `--method <id>` 选择。
terminal 方法直接在当前终端运行登录；agent 方法通过 ACP `authenticate` 完成。
命令行与 GUI 共用原始认证描述符和执行计划。交互式终端可回答 Agent 的表单或确认
授权链接；非交互输入会取消表单，不自动打开网站。认证完成后重新检测会话可用性。
检查只验证 initialize 与 session/new，不发送模型请求；实际对话仍可能遇到模型或网络错误。

安装目录、版本记录和 Registry 缓存由 daemon 管理。优先选择当前平台 binary，
其次使用 npm；npm 安装隔离在后端版本目录，Node 优先复用 daemon 环境中的可用版本，
缺少时下载经过校验的私有 runtime。Pi 的 npm 安装包含它需要的 Pi 本体。
目前 uvx 分发仍会显示在目录中，并标记暂不支持安装。

再次安装同一 ID 可更新版本。新版本采用独立启动路径，现有会话继续使用原进程；
旧版本目录保留，避免影响正在运行的会话。`bast agent remove <id>` 或 GUI“移除”
只移除托管注册和安装记录，保留版本目录、原生凭据和会话记录。
独立安装的自定义后端不会被托管安装覆盖。

安装和认证要求连接本机 daemon。使用 `--agents` 或旧 `--agents-file` 覆盖目录时，
Registry 可浏览，但托管安装不可用；使用统一 settings 文件即可启用。
GUI 会核对 daemon 的 settings 路径，路径不同或远端连接时不修改本机后端配置。
Registry 无法刷新时使用上次有效缓存并显示提示；没有缓存时仍显示已配置后端。

官方目录与格式见 [ACP Registry](https://github.com/agentclientprotocol/registry)。
Registry URL、数据目录和 ELF 修补环境变量见下文；这些选项适合测试、镜像或系统打包，
日常使用无需设置。

## 配置 ACP 后端

GUI 的外观与服务连接保存在 `$XDG_CONFIG_HOME/bas/settings.json`；
后端 Agent 注册与 ACP 策略保存在 `$XDG_CONFIG_HOME/bas/daemon.json`。
未设置 XDG 时，Linux 使用 `~/.config/bas/`。
GUI 使用 `--settings-file` / `BAS_SETTINGS_FILE`，daemon 使用
`--daemon-settings-file` / `BAS_DAEMON_SETTINGS_FILE`，两者独立选择路径和校验内容。
daemon 暂时保留显式 `--settings-file` 兼容参数，但不再读取前端的 `BAS_SETTINGS_FILE`；
旧服务环境请改用 `BAS_DAEMON_SETTINGS_FILE`。GUI 的 Agent 设置通过所连接服务的 RPC 修改后端配置，
不是写入 GUI 偏好文件。以下为 `daemon.json`：

```json
{
  "agents": [
    { "id": "embedded", "kind": "embedded" },
    {
      "id": "codex",
      "kind": "acp",
      "description": "用于代码审查与实现",
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
  ],
  "acp": {
    "idle_timeout_secs": 1800,
    "default_agent": "codex",
    "pool_size": 1,
    "pools": { "opencode": 2, "pi": 0 }
  }
}
```

`id` 是 GUI 中显示的后端名，必须唯一；`embedded` 是本机 echo 测试后端。
添加其他 ACP 后端时，使用 `kind: "acp"`，把启动程序与参数分别放在 `command.program`、
`command.args` 中。程序可用绝对路径；参数数组保留空格边界，不经过 shell 解析，也不会展开 `~` 或 `$HOME`。
`description` 是自定义智能体的可选备注，可在 GUI 中编辑并显示于卡片，不影响启动参数。
使用备注前须同步更新 Bas daemon；旧版采用严格配置解析，不能识别这个新增字段。
`env` 可为该后端覆盖非秘密环境变量；其余变量继承自 bas。凭据使用各 Agent 的原生
登录或服务环境，不应把 API key 填入共享 JSON。设置页保留已有 `env`，但不收集密钥。

ACP 后端可设置 `cancellation_grace_ms`，表示发送取消后等待本轮结束的宽限期，
单位为毫秒，必须大于零，默认 `5000`。例如需要较长工具清理时间的后端可以设置
`"cancellation_grace_ms": 15000`。超过期限仍会判定后端不可继续并回收进程；
该选项不会关闭强制清理机制。

选择目录的优先级为：`--agents` / `BAS_AGENTS` 简写目录、显式
`--agents-file` / `BAS_AGENTS_FILE` 旧格式数组文件、后端 `daemon.json`。
同一个参数的命令行值优先于环境变量。
显式指定的文件不存在、JSON 无效、重复 ID 或 ACP 缺少启动程序都会使启动失败；
默认 `daemon.json` 不存在时只读导入同目录旧 `settings.json` 的 `daemon`，
保留其中的 Agent 与 ACP 策略；旧统一文件也不存在时才导入 `agents.json`，都不存在才使用内置后端。
首次成功保存 Agent 配置生成独立 `daemon.json`，不删除或改写旧文件。
新文件一旦被运行中的服务观察到，格式错误或消失都不会退回旧配置源。
显式路径不查找相邻旧文件；显式指向旧统一文件时只更新其中的 `daemon`，
保留其他字段。前端保存旧统一文件也会保留其不再解析的 `daemon` 字段。

`daemon.json` 的 `acp` 是服务端策略，不读取 GUI 的默认或上次使用 Agent。省略时
`idle_timeout_secs` 为 `1800`（空闲 30 分钟回收），`pool_size` 为 `1`，
无默认 Agent 或显式池。默认只为最近使用的 ACP Agent 保留一个备用连接，不为所有 Agent 预热。
上例沿用 1800 秒空闲回收；`pool_size` 为 `default_agent` 保留预热进程，
未指定默认 Agent 时使用服务端上次使用的 ACP Agent。`pools` 为各 Agent 设置独立目标，
显式条目优先于自动目标，`0` 表示该 Agent 不预热。所有目标必须是已配置 ACP Agent，
总预留数量上限为 32；超大值、空 ID、未知字段和零秒超时会被拒绝。

显式配置优先于默认值：`"idle_timeout_secs": null` 关闭空闲回收；
`"pool_size": 0` 关闭自动预热，但不会关闭 `pools` 中单独配置的备用连接。
要完全关闭预热，还需将 `pools` 设为 `{}`。已有文件中显式保存的 `null`／`0` 不会被新默认值覆盖。

这里的“上次使用”是成功创建并注册 ACP 会话或接受用户回合时记录的 Agent，
不包括查询历史、改标题、连接检测、预热和后台通知。默认 `localdb` 在创建注册成功后记录选择，
prompt 则将选择与接受事件原子提交。尚未注册就被取消的创建不改变选择。
记录写入不依赖 RPC 响应是否送达；并发操作按选择记录的存储提交顺序排列，
不按客户端收到响应的顺序排列。重启保留最后一次已提交的选择。
创建后的选择写入失败只告警，不撤销已注册的会话；预热继续使用上次有效选择。
它属于服务端数据库，不写回客户端设置；`memory` 模式则只保留到本次 daemon 退出。

### daemon 重启时的 ACP 启动范围

重启先恢复会话元数据，不按历史会话数重新启动进程。后台预热只按以下规则计算备用数量：

1. `pools` 中明确指定且数量大于零的 ACP Agent。
2. 自动目标优先选 `default_agent`；没有配置时读取数据库里的最近使用 Agent，
   但它必须仍在当前目录中且仍是 ACP 后端。
3. 自动目标使用 `pool_size`；同 ID 的 `pools` 条目优先（包括 `0`），两者不相加。
4. 没有有效最近使用记录时不猜测替代 Agent；显式的默认目标和独立池仍可预热。

例如默认配置下，上次使用的是 A，数据库中有 A 的十个会话和 B 的五个会话：
重启只预热 **一个尚未绑定会话的 A 连接**，不拉起十五个旧会话进程。
首次访问需要后端的操作才按原 ID load／resume；查看列表、阅读历史不触发恢复。
停止、归档和失败的会话不会自动启动；重启前未完成的回合沿用中断记录，不自动重发 prompt，
避免重复执行工具或外部操作。预热在后台进行，不等待它完成才开始服务。

首次从旧数据库升级时没有精确的最近使用记录，不从会话更新时间或历史正文猜测；
下一次实际使用后会开始记录。若希望此时立即预热，请显式配置 `default_agent` 或 `pools`。

### 空闲回收与备用连接

GUI 可在“设置 → bas · 后端 → 会话管理”中编辑以下策略。
页面跟随当前后端选择，有效修改自动保存并生效；旧版 bas 须先升级才能提供编辑接口。

空闲回收每 5 秒检查一次，只卸载支持 `session/load` 或 `session/resume` 的 ACP 进程，
保留 Bas 会话 ID、历史及实时订阅。下次发送请求时恢复原 ACP 会话，不创建替代对话。
前台回合、进行中的 ACP 请求、待审批／用户交互，以及未确认空闲的子会话都会阻止回收。
普通客户端订阅和历史阅读不会阻止回收。回收先停止接纳新操作、提交已排队的事件，
再释放进程，不发送终止会话的 `session/close`。Agent 的上下文恢复仍依赖其声明的恢复能力；
协议未报告的后台任务无法由 Bas 推断；此类后端应显式将 `idle_timeout_secs` 设为 `null`。

池大小表示**已初始化、尚未绑定业务会话的备用连接数量**，不是运行中会话数上限。
创建／恢复会话会独占取走一个备用连接，并在后台补齐；用过的连接不回池，避免上下文串用。
预热提前完成进程启动和 `initialize`，仍需正常执行 `session/new` 或 load／resume。
备用进程本身也占内存，建议从每个常用 Agent 一个开始；缩小目标、改动 Agent 启动配置
或关闭 daemon 会释放相应备用连接。预热失败会退避重试，不阻止普通冷启动。

代码中使用同一 `bas::config::AcpLifecycle` 类型配置 daemon 的 `Config.acp`，
或经 `Registry::set_acp_policy` 校验后更新；`Default` 与配置文件省略字段的行为一致。

文件配置每秒自动检查，目录和 ACP 策略整体校验成功后原子替换；
非法更新或已读取文件消失时保留上次有效目录和策略。
新建会话使用新目录；运行中的会话保留其启动配置，空闲回收和预热采用新策略。
`--agents` / `--agents-file` 仅覆盖目录，统一 settings 中的 ACP 策略仍会读取和热重载。
这是旧版“显式目录会完全跳过 settings”的行为变化：使用目录覆盖时，显式指定但不存在、
损坏或策略无效的 settings 文件也会使启动失败，不会静默禁用回收／预热策略。
GUI 自定义配置仅编辑核对过的本机 settings；托管安装由 daemon 保存并注册。
保存自定义配置成功不等于远端或旧版本 daemon 已采用该配置。

`ListAgents` 只返回当前可找到可执行文件的工具条目；缺失、目录或无执行权限的 ACP 程序会被隐藏，
但配置文件仍保留，安装工具或修正路径后下次刷新即可重新出现。
查找与实际启动共用 daemon 的 PATH（含该 Agent 的 `env.PATH` 覆盖），不使用 GUI 的 PATH。
设置页“检测”验证 ACP 握手和临时会话；程序可用、上次检测结果与登录需求分别显示。
检测结束不会保持连接，也不验证模型请求。登录是单独的认证操作，检测按钮不会启动登录。
使用此检测功能需要同时升级 `basg` 与 `bas`，旧版 daemon 会提示不支持。

修改 Agent 目录不需要重启。升级 daemon 可执行文件仍需重启；新版默认使用 `localdb`，
重启后可读取新建会话的历史并按 Agent 能力恢复对话。**旧版只存于内存的会话不能迁移**，
升级时重启仍会停止这些会话；不要以重启作为设置保存动作。

当前 SQLite 版本为 schema 8。部署前备份数据库并预留迁移所需的磁盘空间；
仅支持 schema 5／6／7 的旧 bas 不能直接打开升级后的文件，回滚程序时应同时恢复对应备份。
schema 7 在事务内将旧事件及已有压缩块转入统一的 `history_chunks`，删除旧历史表；
保留原始正文、游标和去重回执，不重新压缩旧块。首次启动可能因数据量较大而耗时，
首次访问会话时会重建段目录。失败整体回滚，不丢弃旧历史。
schema 8 为段目录增加有界提示词预览与时间字段，仅清除并重建派生索引，不改写正文 chunks。
完整回合导航与直接定位接口需要同时更新 bas 和 basg；旧服务端不会回退为不完整的标记列表。
详见 [完成回合的物理存储](rpc.md#完成回合的物理存储)。

## Codex

按 [Codex ACP 官方安装说明](https://github.com/agentclientprotocol/codex-acp#installation)
独立安装 `@agentclientprotocol/codex-acp`。例如在满足上游运行环境要求后执行：

```sh
npm install -g @agentclientprotocol/codex-acp
command -v codex-acp
codex-acp cli login
```

上游适配器自身附带兼容的 Codex CLI；其版本、升级和认证由上游工具及用户管理，
Bas 不锁定、注入或覆盖 CLI 路径。需要自定义 CLI 时遵循上游的 `CODEX_PATH` 说明。
已有 Codex 登录可继续使用；登录命令应在运行 bas 的同一用户下执行。

把 `command -v codex-acp` 得到的、独立安装的可执行文件绝对路径填入
`command.program`，`command.args` 为 `[]`；确认 daemon 的 PATH 能找到它时也可写 `codex-acp`。
全局 npm 的安装目录不一定在 systemd 用户服务 PATH 内；不要把 shell 中能启动等同于服务能启动。
保存后可执行 `bast session create --agent codex --cwd "$PWD"`，
或在 GUI 的 Agent 选择器中选 `codex`。模型等配置选项与 `/` 命令由后端提供。
模型与认证继续使用现有 Codex 配置，bas 不生成另一套凭据或修改 Codex 权限设置。
适配器实现见 [codex-acp](https://github.com/agentclientprotocol/codex-acp)，
底层接口见 [Codex App Server 文档](https://learn.chatgpt.com/docs/app-server)。

`codex-acp 2.1.1` 的权限 **Mode** 与 **Collaboration mode**（Default／Plan）独立。
其默认权限预设为 **Auto review**，每次发起回合都会显式传入审批与沙箱设置，
覆盖 Codex 全局配置中的同名值。若要由用户审批，选择 **Workspace access**
（允许工作区编辑，越界写入与联网请求审批）或 **Read-only**；
**Full access** 使用 `approval_policy=never`，不会弹出工具审批。
配置字段含义见 [Codex 配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)。
权限预设及环境变量读取见
[`codex-acp 2.1.1` 的 AgentMode 定义](https://github.com/agentclientprotocol/codex-acp/blob/68d7d2d5ddfc0ed5746f9f6130892dda685e65dd/src/AgentMode.ts)。

适配器在新建及恢复会话时重新读取 `INITIAL_AGENT_MODE`，默认值为 `agent`。
可在 `bas.env` 中加入以下非秘密环境变量，使权限初始值为人工审批的工作区模式：

```sh
INITIAL_AGENT_MODE=workspace-write
```

修改服务环境文件后需重启 bas；已有 Agent 进程保留启动时的环境。
其他可选值为 `read-only`、`agent`、`agent-full-access`。
自定义 Agent 也可在 `daemon.json` 的该 Agent `env` 中设置；
修改目录安装记录以外的 Agent 配置会使它成为自定义注册，后续目录更新将报告注册冲突，
因此目录管理的安装宜使用服务环境设置默认值。

该版本将同步 `request_user_input` 转为 ACP 表单，但未映射
`request_user_input_async` 的 AgentMessage `questions`；Agent 文本中的“已发出选择题”
不表示 ACP 客户端收到表单。这属于适配器侧兼容限制，等待上游修复，详见
[前台交互与后台任务](gui.md#前台交互与后台任务)。

### 从旧版捆绑安装迁移

`--with-codex` 已移除，使用它会在任何安装动作前报错并指向本文；改用
`make install-bas-service`。更新 Bas 不会卸载旧适配器，也不会修改已有配置。
以前安装的 `~/.nix-profile/bin/codex-acp` 可能仍然存在，已有配置也可能继续指向它。
请按官方文档独立安装适配器，检查 `command -v codex-acp` 的结果和服务实际使用的路径，
再自行把后端命令改为所选安装路径。旧 profile 条目由用户决定是否保留；
本次迁移不自动删除任何包、凭据或配置。已有 `CODEX_PATH` / `env` 同样保留，请自行确认是否仍需要。

## OpenCode 与 Pi

按 [OpenCode 官方安装说明](https://opencode.ai/docs/) 独立安装 OpenCode；
它原生支持 ACP，使用 `opencode acp`，见[官方 ACP 文档](https://opencode.ai/docs/acp/)。
配置程序为 `opencode`（或其绝对路径），参数为 `["acp"]`。

Pi 通过 GUI 的 ACP Registry 目录安装并登录，或使用：

```sh
bast agent install pi-acp
bast agent login pi-acp
bast agent check pi-acp
```

Registry 安装负责适配器及其所需的 Pi 本体，成功后自动注册，不需要全局 npm 安装、
仓库开发 shell 或手写模型配置。认证沿用 Agent 原生流程。
已有独立安装的 ACP 程序仍可通过上文的通用自定义配置接入；Pi 原生的
`pi --mode rpc` 不是 ACP，需使用 [pi-acp 适配器](https://github.com/svkozak/pi-acp)。
`pi-acp 0.0.34` 仅在 Pi 扩展主动调用 `ctx.ui.select()`／`confirm()` 时转发
ACP 权限请求；普通助手文字中的选项和内置工具执行不会自动弹卡。
如需测试这种请求，可参考 [Pi 官方审批扩展示例](https://github.com/earendil-works/pi/blob/v1.1.0/packages/coding-agent/examples/extensions/permission-gate.ts)。
该适配器尚未将扩展的 `input`／`editor` 转为 ACP 表单。
配置命令不执行安装、下载、登录或模型请求。
如果服务找不到程序，可填写绝对路径；GUI 自身的 PATH 不代表 systemd 服务的 PATH。

## Registry 与运行环境覆盖

| 环境变量 | 默认 / 用途 |
| --- | --- |
| `BAS_ACP_REGISTRY_URL` | 官方 Registry CDN；允许 HTTPS，HTTP 仅限 loopback 测试服务 |
| `BAS_ACP_DATA_DIR` | 用户数据目录下 `bas/acp`；覆盖时必须为绝对路径 |
| `BAS_ACP_PATCHELF` | 可选的 patchelf 程序路径，Nix daemon 安装包自动提供 |
| `BAS_ACP_ELF_LOADER` | 托管 ELF 的动态解释器，Nix 安装包自动提供 |
| `BAS_ACP_ELF_LIBRARY_PATH` | 托管 ELF 的运行库路径，Nix 安装包自动提供 |

这些 ELF 选项只作用于 Bas 下载的托管程序，不改动用户独立安装的程序。
从源码运行 NixOS daemon 时可使用系统 Node 和已安装的后端；需要验证下载的 ELF 时，
设置与 Nix 安装包相同的修补环境。

## 服务管理

```sh
systemctl --user status bas.service
journalctl --user -u bas.service -f
systemctl --user stop bas.service
systemctl --user start bas.service
```

用户服务默认随用户会话启动，支持就绪通知、watchdog 与异常退出后重启。
可在 `~/.config/bas/bas.env`（或 `$XDG_CONFIG_HOME/bas/bas.env`）中设置服务环境变量，例如：

```ini
BAS_LISTEN=local
BAS_HISTORY=8192
```

服务的 PATH 包含 `~/.local/bin`、`~/.nix-profile/bin` 和系统软件路径；
其他位置的 ACP 程序应在 `daemon.json` 的 `agents` 中使用绝对路径。此运行时环境文件不进入 Nix 安装包。

同名版本更新时重新运行安装目标即可；跨改名升级先按 [手动迁移说明](rename-migration.md) 处理。
脚本会替换同名的旧 profile 条目，`install-bas-service` 还会重启
用户服务，让新可执行文件立即生效；后端配置保留。

```sh
make install-bas-service
make install-basg
```

新安装的 daemon 默认将会话元数据和历史持久化于用户状态目录；不需要持久化时可在环境文件中设置
`BAS_STORAGE=memory`。旧版内存会话在第一次重启时仍会丢失；兼容版本之间只换 GUI 时不需要重启服务。
卸载 Bas 执行 `nix profile remove bas basg`（未安装的条目可省略，实际名称用
`nix profile list` 查看），再 `systemctl --user disable bas`。GUI 的安装、更新与卸载见
[GUI 说明](gui.md)。外部工具由用户单独管理，不随 Bas 卸载。
