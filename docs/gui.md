# BASR 图形客户端

`basrw` 使用 GPUI Kit，代码位于 `src/bin/basrw/`，通过 gRPC 连接 basrd。

窗口激活时会向 basrd 报告当前会话，该会话完成时不弹桌面通知。
切换到其他会话、打开设置、窗口失焦或关闭后恢复后台通知；关闭窗口不会取消任务。
此行为需要同时更新 basrw 和 basrd，旧版守护进程仍会照常通知。
重新打开会自动附加到上次选择的 thread，并恢复它所属的工作目录和 Agent。
选择记录按 basrd 地址保存在 `$XDG_STATE_HOME/basrw/`（默认 `~/.local/state/basrw/`）。
首次使用或原 thread 已被删除时，优先打开最近使用的可用会话，没有可用会话才创建新的 thread。
`w` 表示 window。默认构建不启用图形依赖。

## 设置窗口

workspace 按钮左侧的齿轮或 `Ctrl+,` 打开设置，完整窗口与最小模式都可使用。
这是同一进程中的独立窗口：再次打开只激活已有窗口，不改变聊天窗口尺寸、模式或草稿，
也不会切换或重建 thread。关闭聊天窗口后设置仍可使用，最后一个窗口关闭才退出应用。
标题栏关闭按钮或 `Ctrl+W`（macOS 为 `Cmd+W`）只关闭设置窗口。

左侧分类导航和搜索由通用设置入口负责，具体配置由分类页面负责；
“外观”是可展开的父分类，主题、字体和布局是缩进的子节点。
“Agent 服务”提供本机 ACP agents 配置，后续类别不需要塞进外观表单。
尚未实现的终端、模型供应商等功能不显示占位开关。

字体选择将 `Default`（继承桌面或内置配置）与具体字体放在同一个可搜索下拉框内，
已配置但当前未安装的字体名称不会被悄悄删除。主题来源、模式、窗口背景也提供默认选项。
数字留空表示继承，并有恢复默认入口；字号和行高必须大于零，不透明度范围为 0–1。
每行 `Default`、恢复全部默认和关闭设置使用带底色、细边框的操作按钮；
分类导航仍使用轻量选中态，与可执行操作区分。
Fallback 用逗号分隔，留空继承，`[]` 表示不指定额外 fallback。

有效的选项更改立即自动保存，连续文本编辑短暂防抖后保存，不再需要单独点击“保存”。
关闭前会尝试提交尚未触发防抖的有效编辑；非法输入或文件冲突不写盘，
保留上次有效配置并显示错误。已应用的更改不会在关闭窗口时回滚。
应用级监听持续检查外部配置变化；主题、字体和尺寸同时更新所有窗口，
不依赖某个聊天窗口仍然打开。

所有设置共用 `$XDG_CONFIG_HOME/basr/settings.json`（未设置 XDG 时为
`~/.config/basr/settings.json`），`--settings-file` / `BASR_SETTINGS_FILE` 可覆盖路径。
外观存入 `appearance`，Agent 目录存入 `daemon.agents`；保存某一 section 不覆盖另一 section。
默认新文件缺失时只读合并旧 `appearance.json` 和 `agents.json`，首次保存写入新文件，旧文件保留。
`--theme-file` 优先于表单中的主题文件；内置来源不会读取外部主题。
保存使用同目录临时文件和原子替换；无法解析的原文件、打开页面后的外部内容变更、
符号链接（例如 Nix 管理的配置）均不会被静默覆盖。
没有未提交编辑时，页面同步外部有效配置；有未提交编辑时保留输入并报告冲突。

Agent 服务页提供 Codex、OpenCode、Pi 和自定义 ACP 模板，分别使用 `codex-acp`、
`opencode acp`、`pi-acp`；不会自动安装或启动。程序与参数分开编辑，参数是 JSON 字符串数组，
不经过 shell。命令修改需点击“保存 Agent”，不随半输入的文本自动提交；未保存草稿可取消。
本机新版 `basrd` 每秒读取文件目录，成功后仅影响新建 thread；现有会话保持原后端。
显式 `--agents` / `--agents-file` 覆盖统一配置时，服务不跟随这里的修改。
页面不远程修改服务、不检测 daemon 的 PATH，也不把“已保存”冒充“已在 daemon 应用”。
可用重新加载读取外部修改，有草稿时需先保存或取消。依赖和认证参见[后端配置](install.md)。

主窗口左上角为应用菜单，不再显示难以辨识的小尺寸 logo。
菜单按“关于 / 查看更新”、“设置”、“退出”分组，以分隔线区分，菜单项不加省略号。
菜单沿用 UI 字体与主题选中态；右侧快捷键提示来自实际注册的按键绑定，不是静态文字。
应用内 `Ctrl+,` 打开设置、`Ctrl+Q` 打开退出确认（macOS 使用 `Cmd`），独立设置窗口中也有效。
关于窗口使用较大的 v4 常规线框 logo，
不修改造型或线宽；“查看更新”打开项目发布页面供手动检查，不会自动下载或安装。
退出需确认：未发送草稿和本地队列会丢失，后台已提交任务继续；有效的设置编辑会先保存。
桌面启动器使用拟物 SVG，安装到 hicolor 图标主题；Nix 桌面入口引用安装目录的
绝对图标和可执行文件路径，避免桌面进程的主题缓存或 PATH 未更新导致缺图标。
所有图标资源随应用提供，不依赖原始设计目录或网络。

## 安装

Linux / NixOS 可在仓库根目录安装到当前用户的 Nix profile：

```sh
./scripts/install-basrw
basrw --server 127.0.0.1:7337 --cwd /path/to/project
# 完整窗口
basrw --window
```

安装脚本构建 release 版本，包含尚未加入 Git 的源码，并排除 `.env`、
测试目录和构建输出。安装包包含运行时图形库的启动包装与 `BASR` 桌面菜单入口，
日常启动不需要 `nix develop`。桌面标识仍为 `org.basr.Gui`，现有窗口规则可继续使用。
安装不启动 basrd；服务可用 `./scripts/install-basrd --with-codex --service` 安装，见 [安装与后端配置](install.md)。
卸载使用 `nix profile remove basrw`；安装新的本地版本时，移除旧项后重新运行安装脚本。

源码均已加入 Git 后，也可以直接使用 flake 输出：

```sh
nix profile install .#basrw
# 或只构建 / 临时运行
nix build .#basrw
nix run .#basrw -- --window
```

NixOS / Home Manager 的 flake 配置中，将
`inputs.basr.packages.${pkgs.stdenv.hostPlatform.system}.basrw` 加入
`environment.systemPackages` / `home.packages`。

## 本机启动

在仓库的 `.env` 配置 `DEEPSEEK_API_KEY` 后：

```sh
# 最小弹窗：同时启动 basrd、pi / DeepSeek 和 GUI
./scripts/dev-local

# 直接打开 workspace / thread 窗口
./scripts/dev-local gui --window

# 使用你的项目目录
./scripts/dev-local gui --cwd /path/to/project
```

这个测试入口在 GUI 关闭时一起停止 basrd 和 pi。希望关闭 GUI 后 Agent 继续运行，
先运行 `./scripts/dev-local daemon`，再单独打开客户端：

```sh
nix develop .#gui -c cargo run --features gui --bin basrw -- \
  --server 127.0.0.1:7337 --cwd /path/to/project

# --window 直接进入窗口模式；--compact 是默认最小模式的显式写法
nix develop .#gui -c cargo run --features gui --bin basrw -- --window
```

`BASR_SERVER` 也可以指定地址，命令行参数优先。完整本机测试说明见 [local.md](local.md)。
非 Nix 环境需要 Rust 和 GPUI 的字体、Wayland / X11、图形系统依赖，参考
[GPUI Kit 安装说明](https://gpui-kit.com/docs/installation/)。

## 两种模式

最小模式约 640 × 95，高度由状态行、编辑行数和操作栏共同决定，底部保留 2px 内边距。
输入框不保留代码编辑器的末行滚动留白，空白或单行内容不会产生纵向滚动条。选择 Agent 后即初始化 thread，
拿到选项和命令后即可调整配置，无需先发送消息；Agent 输出在输入框上方展开到约 640 × 440，独立滚动，可以随时折叠。
发送保留当前折叠状态；折叠不停止 Agent，状态行显示思考、工具调用或输出进度。
完成后显示最终回复首行的开头，连接指示灯的黄色描边表示有未读回复。点击摘要或展开输出会定位到最新回复；
只有窗口在前台且最新回复已展示时才清除未读标记，同一窗口内切换会话和重新同步保留已读状态。
悬停摘要可预览完整消息块，长回复支持滚动；预览使用独立弹窗，不抢输入焦点，也不会清除未读标记。
最小模式中，上方折叠按钮与下方附件按钮对齐，上方最大化按钮与下方发送按钮对齐。
每种模式只保留一处连接指示灯：黄色表示正在连接或同步，绿色表示已连接，红色表示断开或会话不可用；
悬停可查看地址与具体状态，空闲时不占用文字空间。连接未就绪或 thread 尚在创建时禁用发送，
按钮与 Enter 使用相同的检查，草稿会保留。
右侧展开按钮切换到完整窗口，保留当前 thread、草稿与对话。两种模式都显示用户消息与 Agent 输出；
多行输入与命令补全菜单会增加弹窗高度。完整窗口的同步状态、滚动按钮与输入框共用居中的内容宽度。

两种模式共用输入框下方的一排工具栏：左侧是附件加号和 Agent 图标，右侧是选项当前值、
开关图标和发送 / 停止按钮。选项过多时横向滚动，发送按钮始终可见。图标与选项提供悬浮提示。
最小模式下，Agent、模型等选项和 workspace 选单使用独立的原生弹出窗口，
可越过输入窗边界；Wayland 合成器负责在屏幕边缘翻转或平移菜单。
长列表限制高度并支持滚动和键盘选择，Esc、选择条目或点击外部关闭菜单，输入窗保持原高度。
不支持原生弹出窗口的平台会临时展开窗口容纳选单，关闭后恢复。
附件加号打开文件选择器，也可直接拖入文件；拖入时输入框高亮。
Ctrl+V 支持剪贴板图片和从文件管理器复制的文件，普通文本仍按编辑器默认方式粘贴。
附件在正文中显示为紧凑的图标标签，图片显示为 `Image`，悬停可预览；预览可越过小窗边界。
标签可以整体选择、删除、撤销和重做，文件内容随 thread 草稿保留，发送失败不会清空。
发送时按正文中标记的位置构造 ACP 内容块：图片使用 `ImageContent`，支持 `embeddedContext` 时文件使用原生嵌入资源，
否则 `.nix` 等文本文件使用带文件名和 URI 的原生文本块。不会把界面标记发给 Agent。
不支持的图片或二进制文件在插入前拒绝；单文件最多 8 MiB、一次最多 16 MiB，可单独发送附件。
错误只在发送 / 停止按钮的悬浮提示上方显示；操作错误在继续编辑或开始新操作时清除。

处理消息时可以继续输入，Enter 或发送图标将新消息加入本地待发送队列，当前回复结束后依次发送。
时钟图标显示待发送数量，点击可取回编辑；红色停止按钮取消当前任务并暂停后续发送。
发送失败的追加消息会保留，等待取回重试。待发送队列属于当前 GUI 实例，关闭窗口不会继续发送队列中尚未提交的消息；已经提交给 basrd 的任务继续运行。
多行工具命令收起时显示单行摘要，悬停及展开后保留原始换行，右侧执行状态始终可见。

窗口模式以工作目录分组：一个 **workspace 对应 basrd 上的一个 cwd**，组下的
**thread 对应一条 session**。同名但路径不同的目录不会合并。组可以折叠，可在组内
新建 thread；文件夹加号添加其他工作目录。当前版本不提供多根目录 workspace。

左侧顶部为 BASR 应用菜单，不显示 Threads 标题或主题名称；主题只在设置页查看和修改。
左侧支持按标题、Agent、路径或 ID 搜索，Enter 打开第一个结果。
小窗和大窗都支持 `Ctrl+Tab` 切换下一个 thread，`Ctrl+Shift+Tab` 切换上一个；
按当前搜索与归档筛选后的分组列表顺序循环，保留各 thread 草稿，不展开小窗或新建 Agent。
点击右侧标题可在原位置编辑，Enter 保存、Esc 取消；失败保留输入。
搜索框外侧右边的归档图标切换“已归档”列表，选中态表示当前正在查看归档。
thread 行右侧与 workspace 的加号共用固定操作列：启动和运行时为 12px 旋转圆环，
未读回复为 6px 主题强调色实心点，就绪且已读时留白；失败显示警告，已停止显示低对比短横。
减少动态效果时遵循组件的 reduced-motion 策略，不持续请求动画帧。
鼠标移到该行时改为归档 / 恢复图标，不改变标题宽度或行高。
右键菜单可重命名、归档 / 恢复、置顶 / 取消置顶，以及永久删除。
未读会话还可“标为已读”，用于主动确认回复或清除已被历史容量淘汰的未读提示。
置顶按当前 basrd 地址保存在本机窗口状态文件中，在 workspace 内优先显示，
重开 GUI 后保留，`Ctrl+Tab` 与显示顺序一致。
归档从普通列表隐藏会话，可以在归档列表查看历史并恢复；
普通搜索和启动时自动恢复不包含归档会话。
归档不停止 Agent，已接受的回合继续执行；需要中止时仍可手动取消。
归档会话不能发送新消息，未发送草稿、附件和队列保留，队列暂停。
恢复不会自动重发队列，需主动取回编辑并提交。操作失败保留原状态，不会退化成删除。
永久删除需单独确认，会停止对应 Agent 并清除会话和本地草稿/队列；
也可使用 `basrctl session remove <id>`。归档失败会显示独立错误窗口，
旧服务不支持时明确提示升级及重启的内存数据丢失风险，不会以删除替代。
归档沿用 daemon 的内存存储和历史容量上限，
不是磁盘持久化：关闭 GUI 不影响会话，重启 daemon 仍会丢失会话和归档状态。
输入框下方可以选择 Agent；首次发送前可以更换 Agent 并保留草稿。更换 Agent 或 workspace
会初始化对应会话，迟到的初始化结果不会切走当前 thread。首次发送用首行生成标题，无需创建表单。
初始化失败保留草稿，可用 F5 重试。选项请求完成后才允许发送，确保新回合使用选定配置。

用户输入支持 Markdown 语法高亮，用户消息以细边框区分。消息不显示发送人标题行，复制按钮在悬停时出现。
消息支持粗体、链接、列表、
代码块、选择与复制，HTTP(S) 和邮件链接可打开。Markdown 解析状态随消息保留，解析更新后
重新测量对应行。工具调用在发起位置显示，后续结果原位更新，回答继续追加在工具之后；
思考过程、工具参数与结果可展开。Markdown 样式参考 Zed Agent Panel 的紧凑标题比例、段落间距与代码块边框。
Markdown 中的 HTTP(S) 图片通过应用注册的 HTTP client 加载，使用系统证书验证并遵循代理环境变量，
默认请求总期限为 30 秒。加载会向图片 URL 所在服务器发起网络请求，不会自动附加 BASR
会话内容。Mermaid 目前仍按代码块显示，图形渲染为远期待办。
思考块使用左侧灯泡和淡色标签、右侧展开箭头，展开后的正文沿左侧细线缩进，复制按钮在悬停时出现。
长工具标题会省略显示，悬停查看完整命令；状态图标固定保留在右侧，不随标题长度挤出卡片。
输入框、用户消息、回答、表格和代码块使用内容字号：优先使用 Zed 的 `agent_ui_font_size`，
未设置时使用 UI 字号的 87.5% 并取整（UI 为 16px 时，内容为 14px）。行内代码使用上游的默认字号比例，不再通过本地库补丁强制与正文等大；表格可横向滚动。
代码块复用 GPUI 的 Tree-sitter 高亮，行内代码使用正文前景色和浅底色。
Markdown 解析、排版、换行与选择全部使用原版 GPUI Base，不维护本地源码覆盖。
移除覆盖后确认的输入框问题，以及混合字体、行内代码布局和桌面集成的待复核项，
见[原版 UI 依赖基线](upstream-ui.md)。工具栏继续使用 UI 字号。
终端结果支持 [Zed 的终端输出扩展](https://github.com/zed-industries/zed/blob/main/crates/agent_servers/src/acp.rs)，
流式追加输出并处理 ANSI 控制序列与退出状态；没有终端流时显示 Agent 提供的 `rawOutput`。
只有终端引用而没有实际数据时会明确提示，不再只显示“终端输出”。

输入 `/` 显示 Agent 通过 ACP 发布的命令列表，支持前缀过滤、↑ / ↓ 选择、Tab / Enter 补全、
鼠标选择和 Esc 关闭。补全后显示 Agent 提供的参数提示，再按 Enter 发送。未知命令也按普通
prompt 原样发送。实现遵循 [ACP slash commands](https://agentclientprotocol.com/protocol/v1/slash-commands)，
消息布局与持久 Markdown 状态参考 [Zed Agent Panel](https://github.com/zed-industries/zed/blob/main/crates/agent_ui/src/conversation_view/thread_view.rs)。
输出使用虚拟滚动，默认跟随尾部，向上阅读后停止跟随；窗口模式的向下箭头恢复跟随。
运行时可以停止当前回合，也可提前编辑下一条消息。切换 thread / workspace
保留当前窗口内的草稿；发送失败保留原文。单独运行的 GUI 退出只断开客户端。

| 快捷键 | 操作 |
| --- | --- |
| Enter / Shift+Enter | 发送 / 换行 |
| Ctrl+Shift+M | 最小模式 / 窗口模式 |
| Ctrl+O | 折叠 / 展开最小模式输出 |
| Ctrl+N | 当前 workspace 新建 thread |
| Ctrl+F | 展开窗口并搜索 threads |
| Ctrl+L | 聚焦输入框 |
| Ctrl+B | 展开窗口或切换侧边栏 |
| Ctrl+, / Cmd+, | 打开或聚焦设置 |
| Ctrl+Q / Cmd+Q | 打开退出确认，不停止 daemon |
| Esc | 关闭编辑区；否则缩回最小模式、折叠输出、关闭弹窗 |
| F5 | 刷新列表并重新加载当前 thread |

在 Niri 下，客户端通过 IPC **仅调整自身窗口**：最小模式使用浮动窗口，展开后回到平铺。
不修改 Niri 配置。其他桌面使用 GPUI 的窗口缩放，最终大小和浮动策略由窗口管理器决定。
尚未注册全局快捷键或实现单实例唤起；可以把启动命令绑定到桌面快捷键。

## 跟随 DMS / Zed 主题

客户端每秒检查 DMS 生成的 `$XDG_CONFIG_HOME/zed/themes/dank-zed-theme.json`
及 Zed 的 `settings.json`。未指定 XDG 路径时使用 `~/.config`。
优先采用 Zed 选中的主题变体，支持 JSONC、固定深浅模式、系统模式和 `theme_overrides`。
内容面板默认使用主题背景色、94% 不透明度，并请求窗口背景模糊，保留轻微的磨砂效果。
侧边栏保留主题 panel 背景的 alpha 通道；侧边栏下方没有额外的不透明底色。
实际背景模糊由桌面合成器支持情况决定，正文不降低不透明度。

DMS 更换壁纸或配色后，生成文件的更新会自动生效。文件重写期间的无效 JSON
不会清空当前配色。没有 Zed 主题文件时，回退到
`$XDG_CACHE_HOME/DankMaterialShell/dms-colors.json`；再不可用时采用内置系统深浅主题。
字体优先使用 Zed 的 UI / buffer 字体设置，否则读取 DMS 的字体设置。
中文回退遵循 Zed 的 `ui_font_fallbacks`，默认优先选用已安装的 `Sarasa UI SC`（更纱 UI），
避免 Linux 字体后端把可变 CJK 字体呈现为过细的默认字重；正文、输入框与行内代码沿用相同回退链。

也可通过 `--theme-file /path/to/zed-theme.json` 指定另一份 Zed 主题文件，支持实时重载。
BASR 只读取桌面主题，不修改 Zed 或 DMS 配置。实现参考
[DMS 应用主题](https://danklinux.com/docs/dankmaterialshell/application-themes)、
[Zed workspace / thread 侧栏](https://github.com/zed-industries/zed/blob/main/crates/sidebar/src/sidebar.rs)
及 [Agent thread 视图](https://github.com/zed-industries/zed/blob/main/crates/agent_ui/src/conversation_view/thread_view.rs)。

### 统一设置中的外观偏好

`$XDG_CONFIG_HOME/basr/settings.json`（默认 `~/.config/basr/settings.json`）
保存 BASR 偏好和 Agent 配置；也可用 `--settings-file /path/to/settings.json` 指定位置。
文件不存在时读取旧格式作为迁移来源；没有旧文件则沿用默认，不创建空文件。可在独立设置窗口修改，也可直接编辑 JSON；
两种入口共用校验、合并及热更新规则。
这是严格 JSON：未知字段、非法枚举、空字体名、非正数或非有限字号/行高，以及不在 `[0, 1]`
内的不透明度都会被拒绝。外部 Zed/DMS 文件仍兼容 JSONC 和未知字段。

示例（只填写需要覆盖的项目）：

```json
{
  "appearance": {
    "ui_font_family": "Inter",
    "mono_font_family": "Fira Code",
    "font_fallbacks": ["Sarasa UI SC"],
    "ui_font_size": 16,
    "content_font_size": 14,
    "content_line_height": 1.6,
    "content_opacity": 0.94,
    "background": "blurred",
    "theme_source": "desktop",
    "theme_mode": "desktop"
  }
}
```

| 字段 | 缺省行为 / 可选值 |
| --- | --- |
| `ui_font_family` / `mono_font_family` | Zed UI / buffer 字体，然后 DMS UI / mono 字体，然后运行时平台 fallback |
| `font_fallbacks` | 导入 Zed `ui_font_fallbacks`；显式数组替换导入链，`[]` 清空导入链 |
| `ui_font_size` | 导入 Zed `ui_font_size`，否则 15px |
| `content_font_size` | 导入 Zed `agent_ui_font_size`，否则最终 UI 字号 × 87.5% 取整（最小 1px） |
| `content_line_height` | 1.6 倍正文行高 |
| `content_opacity` | 0.94；只影响正文背景，不降低文字不透明度 |
| `background` | 默认 `blurred`；`auto` 沿用外部主题背景策略，或 `opaque` / `transparent` / `blurred` |
| `theme_source` | 默认 `desktop`；`builtin` 完全不读取桌面主题和字体，使用内置主题再叠加显式偏好 |
| `theme_mode` | 默认 `desktop` 尊重 Zed 选择；`system` 跟随窗口系统模式，`light` / `dark` 固定模式 |
| `theme_file` | 外部 Zed 主题文件路径；相对路径以偏好文件所在目录为基准 |

合并顺序为 **内置默认 → 外部 Zed/DMS 适配 → BASR 显式偏好**。
`--theme-file` 优先于 JSON 的 `theme_file`，仅覆盖外部主题路径，不覆盖其它偏好；
`theme_source: "builtin"` 时外部路径均不使用。CLI 相对路径以启动工作目录为基准。
固定模式覆盖 Zed 的固定主题名/模式，但继续采用对应模式的命名变体和主题颜色覆盖。
使用内置源时，`desktop` 模式等同 `system`。

偏好、选中的外部来源以及系统深浅模式每秒检查一次。每次先读取、解析、校验并完成最终
resolve，全部成功才应用。热更新的 IO、JSON 或校验失败会记录带路径的日志，并保留
**整份上次有效配置**；相同连续错误不重复刷日志。文件删除按缺省/导入重新计算；
权限错误等其它 IO 错误不当作文件缺失。首次启动没有有效配置时使用内置默认。
仍选择 `desktop` 就表示依赖该组导入，坏的桌面配置会拒绝整次更新；
选择 `builtin` 后，坏的桌面文件不会阻止完全显式的 BASR 偏好生效。
建议通过临时文件加原子替换更新配置，避免写入期间的中间状态。

字体存在性检测、去除未安装字体、平台 UI 字体替代，以及末尾追加一个已安装的 CJK
安全回退字体属于 **runtime resolution**，不写回偏好文件，也不是个人桌面的固定路径配置。
因此 `font_fallbacks: []` 清除的是用户/导入链，平台安全回退仍保留。
实际透明与模糊效果取决于窗口合成器；`opaque` 配合 `content_opacity: 1` 可请求不透明外观。
配置模型支持 serde 读写，后续页面可复用同一模型和校验，不需要直接修改外部 JSON。

## 数据与验证

历史按游标分页读取后接实时订阅，断线自动续接，游标过期时重新加载保留窗口。
切换 thread 使用订阅代次隔离迟到消息；较晚完成的初始化请求不会自动发送消息，
也不会抢走用户正在编辑的 workspace。命令目录保存在原生 ACP 会话快照中，不随历史裁剪丢失。修改请求不会自动重试，避免重复执行。

basrd 仅保留有界历史，GUI 不承诺恢复已淘汰的消息或服务重启前的 session。
Agent 提供 `configOptions` 时，输入区展示选择或开关控件，可用于模型、模式、推理级别等设置；选项顺序遵循 Agent 定义。
对话可展示 ACP 图片、嵌入文本文件、资源链接、工具内容和 diff 文本。
目前还没有交互式工具审批、diff 审批或音频播放；RPC 已支持 ACP 的完整内容类型。

```sh
nix develop .#gui -c cargo check --features gui --bin basrw
nix develop .#gui -c cargo test --features gui-tests --bin basrw
nix develop -c cargo test --all-targets
```

测试覆盖分组、两种模式切换、输出折叠、提前初始化、草稿隔离、中文与多行输入、
命令补全、Markdown 流式解析、工具与消息顺序、终端流与结果兜底、历史裁剪后的命令目录保留、
工具栏与内容宽度、折叠状态进度、剪贴板图片和附件删除 / 撤销、文本附件回退、不支持附件的插入拦截与错误清理、
附件草稿与发送失败、归档与恢复、主题选择与透明度，
连接及会话初始化期间的发送拦截，
以及真实本机 gRPC 的附件传输、历史与断线恢复。
无界面测试不替代实际桌面上的输入法、缩放、透明合成和焦点验证。

可用独立的固定内容预览核对桌面视觉，不连接 basrd 或发起 Agent 请求：

```sh
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel empty
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel sample
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel layout
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel connecting
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel disconnected
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel menus
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel files
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel thoughts
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel completed
nix develop .#gui -c cargo run --features gui-tests --bin basrw -- --preview-panel reply-preview
# 加 --window 查看完整窗口布局
```
