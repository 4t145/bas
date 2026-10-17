# BasG 图形客户端

`basg` 使用 GPUI Kit，代码位于 `src/bin/basg/`，通过 gRPC 连接 bas。

窗口激活时会向 bas 报告当前会话，该会话完成时不弹桌面通知。
切换到其他会话、打开设置、窗口失焦或关闭后恢复后台通知；关闭窗口不主动取消回合，
待审批请求按下文的审批订阅生命周期取消。
此行为需要同时更新 basg 和 bas，旧版守护进程仍会照常通知。
重新打开会自动附加到上次选择的 thread，并恢复它所属的工作目录和 Agent。
选择记录按 bas 地址保存在 `$XDG_STATE_HOME/basg/`（默认 `~/.local/state/basg/`）。
首次使用或原 thread 已被删除时，优先打开最近使用的可用会话，没有可用会话才创建新的 thread。
默认构建不启用图形依赖。

## 源码组织

`src/bin/basg/` 按功能归属组织，模块入口使用 `foo.rs`，子模块与自有测试放入 `foo/`。

| 模块 | 职责 |
| --- | --- |
| `workspace.rs`、`workspace/updates.rs` | 工作区构造、协调与后端更新的状态归约 |
| `workspace/state.rs` | 窗口、后端、导航、跨会话数据、选中线程、活动编辑器与重命名状态的所有权 |
| `workspace/navigation/` | 工作目录选择、线程分组、侧栏与行展示、固定项及导航恢复 |
| `workspace/composer.rs`、`workspace/composer/view.rs` | 输入模式与快捷键、输入区测量、布局和事件绑定 |
| `workspace/chat.rs`、`workspace/chat/content.rs` | 对话区域与时间线、消息及工具/终端内容渲染 |
| `workspace/interaction/` | 当前会话的审批、认证与补充信息交互 |
| `agents/` | 跨窗口的 Agent 服务、变更通知、元数据与图标 |
| `settings/` | 设置页面、`agents/registry.rs` 目录浏览器、`window.rs` 独立窗口宿主 |
| `authentication/`、`elicitation.rs` | 共享认证流程，以及认证和会话共用的表单/URL 面板 |
| `ui/` | 通用按钮、提示与输入几何辅助 |

工作区的子目录是同一 `Workspace` 的职责分组，不各自创建实体或复制状态。
跨会话草稿、发送队列与后台操作属于会话存储，不随当前选中线程或输入区切换而清空。
`connection`、`conversation`、`submission` 与 `configuration` 保持独立于页面；
设置页面使用共享配置存储，而不拥有它。设置窗口的生命周期也独立于工作区窗口。

模块白盒测试随所属模块存放；`workspace/tests/` 保留跨模块交互和状态生命周期回归。
模块间仅暴露实际需要的接口，工作区内部接口限制在 `crate::workspace` 范围。

## 界面语言与设计约定

默认使用简体中文。在「设置 → 外观 → 界面 → 界面语言」选择简体中文或 English，
立即应用到已打开的界面与输入框提示，保留草稿。也可在配置文件中设置
`"appearance": { "language": "zh-CN" }` 或 `"en-US"`；旧配置缺省为中文。
语言偏好沿用外观设置的校验、原子保存和外部热更新。

BasG 自有文案使用 Mozilla [Fluent](https://projectfluent.org/)，资源位于
`src/bin/basg/i18n/assets/{zh-CN,en-US}.ftl`，随可执行文件嵌入。
调用使用稳定的语义 ID 和具名参数；数量变化由 Fluent 选择器处理。
简体中文是资源缺失时的回退。资源测试检查解析、消息覆盖、参数一致性及复数规则。
GPUI Kit 内置编辑菜单和选择器通过其公开语言 API 同步切换。
用户消息、路径、智能体名称、智能体提供的选项和服务端原文按原样展示。

界面统一使用「会话 / Conversation」「工作区 / Workspace」「智能体 / Agent」。
标题说明对象，说明文字描述用途；空状态给出下一步操作。操作型提示说明所在控件的动作，
可附快捷键。信息型入口展示当前信息：时间线预览提示词、用量环显示占比与用量、
智能体图标显示当前名称；长标题保留完整预览。错误和进度在对应区域展示，精确用量与费用可点击查看。
同一动作复用同一消息 ID。新增界面文案需同时提供中文和英文。

`ui.rs` 集中定义界面尺度：辅助文字 0.875rem、正文 1rem、分组标题 1.125rem、
页面标题 1.25rem；界面基准字号仍由用户配置。消息与代码保留独立正文字号和行高。
常规控件 32px、紧凑图标按钮 24px，图标使用 16px 或 12px。
按钮按职责区分视觉权重：发送等主要操作保留强调色，工具栏入口使用普通 ghost 样式；
复制、返回内容顶部和展开编辑器使用浅色辅助样式，直接悬停时增强前景与底色，移出后恢复。
同类按钮共用尺度和状态规则，保留主要、辅助、危险、选中与禁用的区别。
菜单复用 GPUI Kit 的选中态与键盘行为，标准宽度 320px、最大高度 360px，
原生弹出窗口进一步受屏幕尺寸约束。提示共用主题颜色、边框、圆角与小号文字，
长内容使用有边界的提示容器。新增字号或尺寸先检查这些尺度是否能满足用途。

## 设置窗口

BasG 应用菜单中的“设置”或 `Ctrl+,` 打开设置，完整窗口与最小模式都可使用。
这是同一进程中的独立窗口：再次打开只激活已有窗口，不改变聊天窗口尺寸、模式或草稿，
也不会切换或重建 thread。关闭聊天窗口后设置仍可使用，最后一个窗口关闭才退出应用。
标题栏关闭按钮或 `Ctrl+W`（macOS 为 `Cmd+W`）只关闭设置窗口。

左侧分类导航和搜索由通用设置入口负责，具体配置由分类页面负责；
“外观”是可展开的父分类，界面、字体和布局是缩进的子节点。
“智能体”提供本机 ACP 智能体配置。
尚未实现的终端、模型供应商等功能不显示占位开关。

外观与会话设置共用表单行布局、控件列宽、字号和分隔线。下拉选择统一使用小号 Select，
文本输入与数字步进器使用相同的紧凑高度；默认智能体不是独立样式的菜单按钮。
会话设置的说明位于左侧，选择控件和确认复选框在右侧对齐。

字体选择将「默认」（继承桌面或内置配置）与具体字体放在同一个可搜索下拉框内，
已配置但当前未安装的字体名称不会被悄悄删除。主题来源、模式、窗口背景也提供默认选项。
界面字体与字号在同一行，字号使用减号、可编辑数值和加号组成的步进器，每次调整 1px。
字号留空时显示当前生效的继承值，不写入覆盖；有自定义值才显示该行的重置图标，
恢复界面字体时同时清除该行的字体和字号覆盖。正文沿用界面字体，正文与代码共用内容字号。
其他数字留空也表示继承；字号和行高必须大于零，不透明度范围为 0–1。
正文行高和不透明度使用同一个步进控件，步长分别为 0.1 和 0.05。
备用字体和主题路径等文本输入也使用左侧的恢复图标，只有存在覆盖时才显示；
不再为这些输入保留另一套文字“重置”按钮。默认的数值输入显示实际生效值。
每行「重置」、「重置外观设置」和「关闭设置」使用带底色、细边框的操作按钮；
分类导航仍使用轻量选中态，与可执行操作区分。
Fallback 用逗号分隔，留空继承，`[]` 表示不指定额外 fallback。

有效的选项更改立即自动保存，连续文本编辑短暂防抖后保存，不再需要单独点击“保存”。
关闭前会尝试提交尚未触发防抖的有效编辑；非法输入或文件冲突不写盘，
保留上次有效配置并显示错误。已应用的更改不会在关闭窗口时回滚。
应用级监听持续检查外部配置变化；主题、字体和尺寸同时更新所有窗口，
不依赖某个聊天窗口仍然打开。

所有设置共用 `$XDG_CONFIG_HOME/bas/settings.json`（未设置 XDG 时为
`~/.config/bas/settings.json`），`--settings-file` / `BAS_SETTINGS_FILE` 可覆盖路径。
外观存入 `appearance`，Agent 目录存入 `daemon.agents`；保存某一 section 不覆盖另一 section。
默认新文件缺失时只读合并旧 `appearance.json` 和 `agents.json`，首次保存写入新文件，旧文件保留。
`--theme-file` 优先于表单中的主题文件；内置来源不会读取外部主题。
保存使用同目录临时文件和原子替换；无法解析的原文件、打开页面后的外部内容变更、
符号链接（例如 Nix 管理的配置）均不会被静默覆盖。
没有未提交编辑时，页面同步外部有效配置；有未提交编辑时保留输入并报告冲突。

Agent 服务页提供官方 ACP Registry 目录，可搜索后端、查看版本和来源，并安装或更新。
目录默认直接展示，提供专用搜索框和“全部 / 已安装 / 未安装”筛选。
目录背景、卡片、边框与操作按钮使用当前主题配色，主题切换或重载时同步更新。
正常条目采用两行布局：名称、图标、版本与主要操作，以及截断描述、ID、仓库与许可证链接；
进度、检测失败详情和登录方式按需展开。自定义后端和底层命令编辑由“高级配置”进入。
目录、thread 侧栏和 Agent 选择器共用官方 Registry 图标。SVG 随主题文字颜色绘制真实轮廓，
位图保留原色；图标按地址缓存，缺失或加载失败时使用通用标识。
聊天 Agent 菜单中的“安装 Agent…”打开同一目录，空目录也有这个入口。
安装状态只显示“已安装 / 未安装”等安装信息，不追加可用性结论。
设置和安装面板共用检查、退出登录和开始对话操作。检查完成后，检查按钮显示可用、
需要登录或失败图标；悬停变为重新检查图标，提示中可查看上次检查结果。
需要认证时单独提供登录方式；检测通过后可用“开始对话”创建实际会话。
安装进度可见，取消或关闭设置会取消尚未完成的安装任务。
安装成功后自动注册、检测并刷新聊天选择器。

安装由 daemon 在独立版本目录执行，GUI 不运行 Registry 中的命令。
binary 优先，npm 包使用隔离安装与可用的系统或私有 Node；uvx 条目暂不支持安装。
更新只影响新启动的后端，已有会话保持原进程。移除托管注册保留版本目录与原生凭据。
自定义 ACP 配置继续支持，程序与参数分开编辑，参数是 JSON 字符串数组，不经过 shell。
路径、参数和取消等待等选项放在配置入口，修改后需要明确保存。

认证按钮显示 Agent 返回的方法名称和说明，执行方式来自原始 ACP 描述符。
terminal 方法打开独立认证终端；agent 方法通过 daemon 持有的
ACP 连接调用 authenticate。终端助手反馈登录进程结果，随后重新检测，
失败时终端保留错误信息，按 Enter 关闭；页面及时显示退出码。终端启动失败也及时显示结果。
不会把终端启动或关闭当成已认证。认证完成并检测通过后自动刷新聊天，
因认证失败保留的草稿会重试创建会话。
设置页和会话页共用认证组件；会话需要认证时直接展示步骤，保留输入与附件。
Agent 提供 Form 时显示字段、默认值与校验错误；提供 URL 时展示地址，由用户打开，
随后等待 Agent 的完成通知。已有会话也支持这些交互；切换会话或断开后不重放旧答案。
凭据继续由 Agent 管理，Bas 不收集或写入共享配置中的 API key。
历史终端描述符由 daemon 对照托管启动记录验证，保留 Node 入口等启动参数。

目录加载会核对 daemon 的 settings 路径。远端服务、不同配置路径、旧版 daemon
或显式目录覆盖会显示对应提示，阻止本机后端编辑和托管安装。
远端认证应在 daemon 所在主机完成。
自定义连接检测使用 daemon 的 PATH 和保存配置快照，并验证 initialize/newSession；
不发送 prompt，不创建聊天 thread，客户端文件、终端和审批回调均拒绝。
检测成功只代表可以创建 ACP 会话，实际模型请求仍可能失败。
详见[安装与后端配置](install.md)。

主窗口左上角为应用菜单，不再显示难以辨识的小尺寸 logo。
菜单按“关于 / 查看更新”、“设置”、“退出”分组，以分隔线区分，菜单项不加省略号。
菜单沿用 UI 字体与主题选中态；右侧快捷键提示来自实际注册的按键绑定，不是静态文字。
应用内 `Ctrl+,` 打开设置、`Ctrl+Q` 打开退出确认（macOS 使用 `Cmd`），独立设置窗口中也有效。
关于窗口使用 96px 的 v4 拟物彩色 logo，与桌面启动器的风格一致。
展开窗口按窗口管理器能力提供最小化、最大化／还原和关闭按钮；macOS 使用原生信号灯。
Wayland 下不显示合成器未提供的最小化、最大化操作，小窗切换仍是另一个操作。
关闭窗口不停止 daemon；“查看更新”打开项目发布页面供手动检查，不会自动下载或安装。
退出需确认：未发送草稿和本地队列会丢失，后台已提交任务继续；有效的设置编辑会先保存。
桌面启动器使用拟物 SVG，安装到 hicolor 图标主题；Nix 桌面入口引用安装目录的
绝对图标和可执行文件路径，避免桌面进程的主题缓存或 PATH 未更新导致缺图标。
应用标识与操作图标随安装包提供；后端品牌图标从官方 Registry 的地址按需读取。

## 安装

Linux / NixOS 可在仓库根目录安装到当前用户的 Nix profile：

```sh
make install-basg
basg --server 127.0.0.1:7337 --cwd /path/to/project
# 完整窗口
basg --window
```

优先使用此快照安装目标构建 release 版本，包含尚未加入 Git 的源码和已初始化子模块，
排除 `.git`、`.delta`、`.env` 等元数据、测试输出和构建输出。
path 快照已有宏源码，不传 Git 专用的 `submodules=1` 参数。
安装包包含运行时图形库的启动包装与 `BasG` 桌面菜单入口，
日常启动不需要 `nix develop`。桌面标识为 `org.bas.Gui`，窗口规则需使用此标识。
安装不启动 bas；服务可用 `make install-bas-service` 安装，见 [安装与后端配置](install.md)。
卸载使用 `nix profile remove basg`；安装新的本地版本直接重跑 `make install-basg`，
脚本会替换同名的旧 profile 条目。

源码均已加入 Git 后，也可以显式使用包含子模块的 Git flake 输出：

```sh
nix profile add "git+file://$PWD?submodules=1#basg"
# 或只构建 / 临时运行
nix build "git+file://$PWD?submodules=1#basg"
nix run "git+file://$PWD?submodules=1#basg" -- --window
```

NixOS / Home Manager 的 flake 配置中，设置
`inputs.bas.url = "git+https://github.com/4t145/bas?submodules=1";`，再将
`inputs.bas.packages.${pkgs.stdenv.hostPlatform.system}.basg` 加入
`environment.systemPackages` / `home.packages`。

## 本机启动

首次克隆或更新后，在物理 checkout 执行 `git submodule update --init --recursive`。
以下 Cargo 和 `nix develop .#gui` 工作流直接读取本地子模块源码。

macOS / Windows 可使用仓库固定的 Rust 工具链原生构建：

```sh
cargo build --features gui --bin basg --locked
cargo run --features gui --bin basg -- --window
```

macOS 也可使用 `nix develop .#gui`，该 shell 仅在 Linux 引入 Wayland 等图形运行库。
Nix 安装包和 systemd 用户服务仍面向 Linux；macOS / Windows 的构建与桌面状态测试
由 CI 的 `native-desktop` job 检查，实机的输入法、缩放与窗口按钮仍需桌面验收。

先在另一个终端运行 `nix develop -c cargo run --bin bas`，或连接已安装的 daemon
用户服务，再启动 GUI。Pi 等外部 Agent 在设置页的 ACP Registry 目录中按需安装，
认证沿用 Agent 原生流程，不需要仓库级模型配置或 `.env`。
GUI 与 daemon 独立运行，关闭 GUI 不会停止 daemon 或后台 Agent。

```sh
nix develop .#gui -c cargo run --features gui --bin basg -- \
  --server 127.0.0.1:7337 --cwd /path/to/project

# --window 直接进入窗口模式；--compact 是默认最小模式的显式写法
nix develop .#gui -c cargo run --features gui --bin basg -- --window
```

`BAS_SERVER` 也可以指定地址，命令行参数优先。服务配置及认证见 [安装说明](install.md)。
非 Nix 环境需要 Rust 和 GPUI 的字体、Wayland / X11、图形系统依赖，参考
[GPUI Kit 安装说明](https://gpui-kit.com/docs/installation/)。

## 两种模式

最小模式宽约 640px，高度由状态行、编辑行数和操作栏共同决定，底部保留 2px 内边距。
输入框使用 GPUI 的字体布局测量软换行，宽度和字号变化后重新测量；紧凑模式最多显示
4 行，完整窗口最多 10 行，更多内容在编辑器内滚动。
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
多行输入与命令补全菜单会增加弹窗高度。完整窗口的状态栏、输入框与输出正文共用同一宽度上限并居中；
输入区仅保留顶部分隔线，不再使用圆角四边框。窄窗口继续按可用宽度收缩。
完整窗口的内容容器统一提供左右 12px 内边距，右侧另为 24px 导航栏留位；
状态、消息与输入框共享边缘，消息卡片与导航栏之间保留 12px 间隙。
侧边快捷跳转按对话顺序集中排列，标记保持固定间距；悬停预览提示词，点击定位对应用户消息。
入口组位于侧边中部，不表达正文的像素位置。最多显示 20 个请求标记，超出时独立滚动，末尾入口始终可用。
最小模式的输入框两侧保留 4px。文字和附件按钮共用编辑器的实际文字起点。
输入框左侧正文和工具栏保留 4px 内边距；应用通过实测起点补偿编辑器自身的额外留白。

两种模式的 Agent 选择器都在输入框上方的工具栏中，位于连接状态灯之前。
选择菜单为每个 Agent 显示与 Registry 一致的图标，当前项在右侧显示勾选标记。
工具栏中的 Agent 图标与相邻按钮使用相同的图标尺寸；设置入口收在 BasG 应用菜单中。
Agent 图标使用 GPUI Kit Badge 显示当前 daemon 中正在工作的后台会话实例数（排除当前选中会话），
以及 ACP 明确公告的活动子 Agent（包括当前会话的子 Agent）。同一 Agent 的不同并行会话分别计数，
不会按工具名字猜测子 Agent。子 Agent 的 `running` 与 `requires_action` 计入数字，
`idle`、`unknown` 或未确认状态不计入；主会话回合结束不清除仍在工作的子 Agent。
空闲、初始化中、已停止及运行时失败的会话不计入；零个时隐藏角标，超过 99 显示 `99+`。
悬停可查看独立浮层，列出 Agent、会话标题、主工作目录和已知工作状态。
浮层可滚动、保持实时状态观察，不切换会话或抢输入焦点；后台条目沿用目录轮询，
不为预览额外建立会话或发送 prompt。连接断开时隐藏数字并明确提示状态不可确认。
输入框下方共用一排工具栏：左侧是附件加号，右侧是选项当前值、
带名称的滑动开关和发送 / 停止按钮。选项过多时横向滚动，发送按钮始终可见。
Boolean 选项直接显示当前开关状态，悬浮提示保留后端说明；提交配置期间禁用，后端确认后更新。
最小模式下，Agent、模型等选项和 workspace 选单使用独立的原生弹出窗口，
可越过输入窗边界；Wayland 合成器负责在屏幕边缘翻转或平移菜单。
长列表限制高度并支持滚动和键盘选择，Esc、选择条目或点击外部关闭菜单，输入窗保持原高度。
不支持原生弹出窗口的平台会临时展开窗口容纳选单，关闭后恢复。
附件加号打开文件选择器，也可直接拖入文件；拖入时输入框高亮。
Ctrl+V 支持剪贴板图片和从文件管理器复制的文件，普通文本仍按编辑器默认方式粘贴。
GPUI 同时提供图片和文本条目时保留两者；仅有文本形式的文件 URI 仍作为普通文本。
这些应用层回归不替代真实 Wayland 合成器的剪贴板 MIME 协商验证。
附件在正文中显示为紧凑的图标标签，图片显示为 `Image`，悬停可预览；预览可越过小窗边界。
标签可以整体选择、删除、撤销和重做，文件内容随 thread 草稿保留，发送失败不会清空。
发送时按正文中标记的位置构造 ACP 内容块：图片使用 `ImageContent`，支持 `embeddedContext` 时文件使用原生嵌入资源，
否则 `.nix` 等文本文件使用带文件名和 URI 的原生文本块。不会把界面标记发给 Agent。
不支持的图片或二进制文件在插入前拒绝；单文件最多 8 MiB、一次最多 16 MiB，可单独发送附件。
错误在编辑区的反馈行显示，可打开详情；操作错误在继续编辑或开始新操作时清除。

Wayland 使用原生 data-control 协议补充 GPUI 的剪贴板读取：支持纯 `text/plain`、
UTF-8 文本、图片与文本并存，以及文件管理器的 `text/uri-list`。
只将本机 `file:` URI 作为附件，忽略远程主机和 HTTP URL，重复路径只添加一次。
macOS 从 pasteboard 同时读取文本、图片和文件；Windows 沿用 GPUI 的多格式读取。
剪贴板图片的说明文本随图片插入，不因 Agent 不支持图片而丢弃文本。
原生补充读取在后台进行，最多等待三秒，期间暂停输入和发送；切换 thread 或改换队列
草稿后，迟到结果会丢弃。当前草稿首次初始化成会话时仍保留粘贴操作。
合成器不提供 data-control，或补充读取失败时，回退到 GPUI 已读取的内容；
该回退的可用格式受 GPUI 与平台支持限制。

设置 → 外观 → 布局中的“消息输入框 Enter”决定发送方式：默认 Enter 发送、Shift+Enter 换行；
选择“Enter 换行”后使用 Ctrl+Enter 发送（macOS 也支持 Cmd+Enter）。设置自动保存，
已打开的消息输入框随之更新，不改变草稿或当前焦点。命令菜单中的 Enter 仍用于补全；
发送快捷键只作用于消息输入框。

输入框右上角的展开按钮或 `Alt+Shift+Esc` 打开更大的多行消息编辑区，再次操作收起。
展开按钮与最右侧发送按钮的中心位于同一竖轴，与首行输入共用高度，不占独立一行。
展开时 Enter 换行，Ctrl+Enter 发送（macOS 也支持 Cmd+Enter）；收起后恢复保存的 Enter
偏好，草稿与附件保留。
完整窗口中的新 thread 默认展开输入框，占满工具栏以下的内容区域；首次发送后恢复对话布局。
Agent 初始化时主动发送的信息显示在独立的“初始化信息”区域，使用 Markdown 渲染；
它不会被当成用户已经开始对话，也不会挤掉大输入框。

处理消息时可以继续输入，发送操作将新消息加入本地待发送队列，当前回复结束后依次发送。
队列显示在对话面板底部，每条消息可修改、撤回或单独发送；编辑后保存保留原队列位置，
Escape 恢复编辑前的输入草稿。时钟图标仍显示数量并可取回队首编辑。
红色停止按钮取消当前任务并暂停后续发送，暂停队列可显式继续。
“立即发送”先取消当前回合，再等待取消成功及空闲状态后提交选中的消息；取消失败或
发送失败时保留消息。单独发送暂停队列中的一条消息不恢复其余消息的自动发送。
待发送队列属于当前 GUI 实例，关闭窗口不会继续发送队列中尚未提交的消息；已经提交给 bas 的任务继续运行。
多行工具命令收起时显示单行摘要，悬停及展开后保留原始换行，右侧执行状态始终可见。
连续的思考与多次工具调用合为一个默认折叠的活动块，显示思考、工具调用和执行中／失败
数量；用户输入和 Agent 输出仍独立显示。展开活动块可查看各条思考和工具详情。
Markdown 复用原版 GPUI 渲染，支持多级列表、列表内代码与表格，并可复制整表为 Markdown。
悬停一个代码块时，仅该块右上角显示“复制代码”按钮；复制正文并保留缩进与换行，
不包含围栏或语言标记。表格下方操作行、消息／思考内容的复制在各自热区悬停时显示。
同一 thread 的 Agent 输出直接显示正文，各段不再重复展示 Agent 名称。
回复图片支持本地绝对路径、相对会话工作目录的路径、`file:`、`data:` 和 HTTP(S)
地址。路径中的空格可使用 Markdown 尖括号目标；加载失败时显示图片占位与简短提示。
每轮完成的回复末尾提供复制本轮回复、返回对应用户消息等操作；最后一轮还可
回到对话顶部。最后是工具或思考时，操作栏放在末尾活动行，折叠时也可使用。
复制保留回复正文的 Markdown，合并本轮多段 Agent 输出，不包含思考或工具输出。
操作平时弱化显示，悬停后增强；最新回复生成中或等待确认时暂不显示。
有序列表非 1 起始编号的上游限制记录在 [UI 依赖问题](upstream-ui.md)。

窗口模式以工作目录分组：一个 **workspace 对应 bas 上的一个 cwd**，组下的
**thread 对应一条 session**。同名但路径不同的目录不会合并。组始终显示展开／折叠箭头，
鼠标移入后显示相邻的新建 thread 和更多操作按钮；整行右键打开同一菜单，支持新建、
展开／折叠、复制目录路径和添加工作目录。文件夹加号也可添加目录。
“添加工作文件夹”唤起系统文件夹选择器，选择后在该目录新建并初始化会话，
通过 ACP `session/new.cwd` 传递目录；取消保留当前线程和草稿。
侧栏添加工作区与给已有会话附加副目录是不同的功能，输入栏不提供前者作为附件操作。
输入栏的文件夹加号管理当前会话的副目录，可附加多个文件夹或移除已附加目录。
仅在空闲且 Agent 声明 `additionalDirectories` 和 load／resume 能力时允许修改。
Bas 使用原 ACP 会话 ID、原主目录，在当前连接上发送 load（优先）或 resume，
携带完整副目录列表；不调用 `session/new`，不上传文件夹内容，不自动停止正在执行的回合。
load 的重复回放不会追加到原历史；成功后保存副目录及返回的配置，并发送有序更新。
副目录在会话重新启动和 daemon 恢复时重新传入。明确拒绝时保留旧配置；
超时或传输结果不确定时停用该运行实例并报错，不把未知目录范围当作修改成功。
文件选择器使用本机路径；远程 daemon 的目录必须在 daemon 主机存在。

工作区右键／更多菜单中的“移除工作区”先确认，再归档该主目录下的全部会话，
不受当前搜索筛选影响，不递归影响拥有不同主目录的工作区。全部归档成功后移除侧栏分组，
保留会话记录、草稿、队列和磁盘文件夹；沿用现有归档语义，先停止运行实例再归档。
部分归档失败时保留工作区并报错，重试只处理尚未归档的会话。
移除状态按 bas 地址保存，归档列表仍可查看这些会话；恢复会话或重新添加目录可再次显示该工作区。

左侧顶部为 BasG 应用菜单，不显示 Threads 标题或主题名称；主题只在设置页查看和修改。
左侧与右侧顶栏共用高度和按钮尺寸，控件垂直居中；搜索框与顶栏之间保留间距。
顶栏项目名与线程标题使用相同字号，重命名输入框保持同行高度。
完整窗口右上角只显示窗口模式、最小化、最大化和关闭等窗口操作；thread 归档使用侧栏操作。
左侧支持按标题、Agent、路径或 ID 搜索，Enter 打开第一个结果。
小窗和大窗都支持 `Ctrl+Tab` 切换下一个 thread，`Ctrl+Shift+Tab` 切换上一个；
按当前搜索与归档筛选后的分组列表顺序循环，保留各 thread 草稿，不展开小窗或新建 Agent。
点击顶栏标题就在顶栏编辑；侧栏的修改标题按钮与右键重命名在对应线程行编辑，
保留当前打开的 thread。失去焦点自动保存，不显示保存／取消按钮；Enter 提交，Esc
撤销本次修改。空白或未变更的标题不提交，保存失败保留尝试内容供下次编辑重试。
搜索框外侧右边的归档图标切换“已归档”列表，选中态表示当前正在查看归档。
thread 标题下显示最近互动时间（刚刚、分钟、小时或天），侧栏可见时每分钟刷新。
线程与本地草稿的 Agent 图标均对齐侧栏左侧留白。
行右侧在鼠标移入后显示相邻的修改标题与废弃草稿／归档按钮；已归档 thread 显示恢复。
尚未初始化的草稿也可修改标题，初始化时会使用该标题。工作区与 thread 的 hover 背景
和右键热区覆盖列表整行宽度，操作按钮包含在该热区内。
时间右侧保留状态指示：启动和运行时为 12px 旋转圆环，未读回复为 6px 主题强调色
实心点，就绪且已读时留白；失败显示警告，已停止显示灰色熄灭灯。
状态指示是展示元素，不是按钮；就绪留白没有独立的 hover、提示或焦点。
Agent 选择旁使用相同的会话状态指示，不用 gRPC 连接状态代替运行状态。
服务器地址与连接状态单独位于侧栏底部，行高和基线与输入框底部工具栏对齐。
减少动态效果时遵循组件的 reduced-motion 策略，不持续请求动画帧。
右键菜单可重命名、废弃草稿或归档／恢复、置顶／取消置顶，以及永久删除。
未读会话还可“标为已读”，用于主动确认回复或清除已被历史容量淘汰的未读提示。
置顶按当前 bas 地址保存在本机窗口状态文件中，在 workspace 内优先显示，
重开 GUI 后保留，`Ctrl+Tab` 与显示顺序一致。
已读回复游标也按服务地址保存，历史重放不会重新标成未读，新的回复仍会显示未读。
连续输出合并写入；切换会话、关闭窗口或退出应用时刷新待保存状态。
旧版本未保存的已读历史无法反推；在此版本中打开或标记已读后，后续重开会保持。
归档从普通列表隐藏会话，可以在归档列表查看历史并恢复；
普通搜索和启动时自动恢复不包含归档会话。
归档会先停止 Agent 和当前回合；恢复会自动启动原会话，成功后才解除归档。
已停止且归档的会话只提供恢复，不另设启动入口；未归档的停止会话可以直接启动。
归档会话不能发送新消息，未发送草稿、附件和队列保留，队列暂停。
恢复不会自动重发队列，需手动继续发送、单条发送，或修改后提交。操作失败保留原状态，不会退化成删除。
永久删除默认在浮动窗口中确认，会停止对应 Agent 并清除会话和本地草稿/队列。
勾选“不再提醒”并确认删除后保存偏好；设置的“会话”分类中可取消该勾选以恢复确认。
会话偏好不混入外观或排版设置，重置外观不改变删除确认偏好。
也可使用 `bast session remove <id>`。归档失败会显示独立错误窗口，
旧服务不支持时明确提示升级及重启的内存数据丢失风险，不会以删除替代。
归档元数据随会话保存；默认 `localdb` 将会话与事件写入本机数据库，关闭 GUI 或重启
daemon 后可以重新读取。`BAS_STORAGE=memory` 仅保留本次 daemon 进程的数据，
已淘汰的内存历史不会恢复。继续原 Agent 对话还取决于该 Agent 的会话恢复能力。
显式新建 thread 自动选择并初始化 Agent：“设置 → 会话 → 新会话默认智能体”优先，
未指定时沿用上一次使用的 Agent。首次启动没有历史时使用目录中的第一个 Agent。
明确从 Agent 卡片“开始对话”时使用该卡片的 Agent，不被默认设置覆盖。
设置的 Agent 不存在或初始化失败时显示错误，不悄悄换成其他后端。
首条用户请求被接受或用户回合开始后固定 Agent；初始化完成但尚未发送用户请求时仍可切换。
切换重新初始化所选后端，保留输入草稿、附件和光标；创建期间禁用，避免同时启动多个初始化请求。
需要更换后端时新建 thread，或从目录“开始对话”。迟到的初始化结果不会切走当前 thread。
首次发送用首行生成标题，无需创建表单。
初始化失败保留草稿，可用 F5 重试。选项请求完成后才允许发送，确保新回合使用选定配置。

用户输入支持 Markdown 语法高亮，用户消息以细边框区分。消息不显示发送人标题行，复制按钮在悬停时出现。
消息支持粗体、链接、列表、
代码块、选择与复制，HTTP(S) 和邮件链接可打开。Markdown 解析状态随消息保留，解析更新后
重新测量对应行。工具调用在发起位置显示，后续结果原位更新，回答继续追加在工具之后；
思考过程、工具参数与结果可展开。工具卡按“输入 → 输出”排列，输入默认收起，输出默认展开；
两者可独立折叠，后续进度更新保留选择。长命令与参数归入输入。
主动展开或收起暂停末尾跟随，保留当前阅读位置；侧边快捷入口的位置不随内容折叠而重新分布。
输入对象中的字符串 `command` 按真实换行显示为命令代码，其余字段独立展示为参数，保留原始 ACP 数据。
执行中的工具、思考、计划及上下文压缩使用旋转状态图标，完成后恢复静态状态；系统减少动态效果设置仍有效。
展开的工具、思考和活动组底部提供返回当前内容顶部及收起按钮，收起后回到对应标题；
同一活动组中的工具分别定位到自己的标题。工具标题按内容字号预留完整行高，保留英文下伸字形。
Markdown 样式参考 Zed Agent Panel 的紧凑标题比例、段落间距与代码块边框。
Markdown 中的 HTTP(S) 图片通过应用注册的 HTTP client 加载，使用系统证书验证并遵循代理环境变量，
默认请求总期限为 30 秒。加载会向图片 URL 所在服务器发起网络请求，不会自动附加 Bas
会话内容。Mermaid 目前仍按代码块显示，图形渲染为远期待办。
活动摘要分别使用思考、工具、运行和失败图标，只有失败项使用错误色，窄窗口自动换行。
思考块使用统一的展开按钮、灯泡和淡色标签，展开后的正文沿左侧细线缩进，复制按钮在悬停时出现。
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
切换会话等待历史时，内容区显示居中的加载指示，不提前显示空会话文案。
匹配当前会话的快照到达后立即显示内容；重连保留已经加载的历史，加载失败显示不可用状态。
会话正文右侧的刻度对应各轮用户提示词，悬停可查看简短提示，点击可跳到该轮；末尾刻度跳到最新输出并恢复跟随。
运行时可以停止当前回合，也可提前编辑下一条消息。切换 thread / workspace
保留当前窗口内的草稿；发送失败保留原文。单独运行的 GUI 退出只断开客户端。

| 快捷键 | 操作 |
| --- | --- |
| Enter / Shift+Enter | 根据“消息输入框 Enter”设置发送或换行 |
| Alt+Shift+Esc（消息输入框） | 展开／收起多行消息编辑器，保留草稿 |
| Ctrl+Enter / Cmd+Enter（macOS） | 发送消息 |
| Ctrl+Shift+M | 最小模式 / 窗口模式 |
| Ctrl+O | 折叠 / 展开最小模式输出 |
| Ctrl+N | 当前 workspace 新建 thread |
| F2 | 重命名当前 thread（小窗会展开以显示编辑器） |
| Ctrl+S | 停止／启动未归档的当前会话，停止不再确认 |
| Ctrl+M | 打开或聚焦会话管理 |
| Ctrl+W（工作区内） | 归档／恢复当前 thread；保留未发送草稿，不关闭窗口 |
| Ctrl+Alt+P | 置顶／取消置顶当前 thread |
| Ctrl+Alt+W | 删除当前 thread；默认确认，遵循“不再提醒”偏好 |
| Ctrl+Alt+N | 选择本机目录以添加 workspace |
| Ctrl+F | 展开窗口并搜索 threads |
| Ctrl+L | 聚焦输入框 |
| Ctrl+B | 展开窗口或切换侧边栏 |
| Ctrl+Home | 跳到当前会话顶部 |
| Ctrl+End | 跳到当前会话末尾并恢复跟随 |
| Page Up / Page Down | 按会话可视区域高度向上／向下翻页 |
| Ctrl+Page Up / Ctrl+Page Down | 跳到上一轮／下一轮用户请求 |
| Ctrl+, / Cmd+, | 打开或聚焦设置 |
| Ctrl+Q / Cmd+Q | 打开退出确认，不停止 daemon |

现有右键菜单和按钮悬浮提示显示对应快捷键，提示从实际按键绑定生成。
悬浮提示统一为“操作 · 快捷键”，由共享格式化入口处理平台按键显示。
会话翻页与逐轮跳转在输入框、搜索框聚焦时同样有效；不改动输入内容。
| Esc | 关闭编辑区；否则缩回最小模式、折叠输出、关闭弹窗 |
| F5 | 刷新列表并重新加载当前 thread |

客户端通过 GPUI 请求窗口类型和尺寸：主窗口使用 `Normal`，设置窗口使用 `Floating`。
最小／窗口模式切换只调整界面布局和尺寸，不调用窗口管理器专用 IPC；最终摆放、平铺与浮动策略由 compositor／窗口管理器决定。
尚未注册全局快捷键或实现单实例唤起；可以把启动命令绑定到桌面快捷键。

## 跟随 DMS / Zed 主题

客户端每秒检查 DMS 生成的 `$XDG_CONFIG_HOME/zed/themes/dank-zed-theme.json`
及 Zed 的 `settings.json`。Linux 未指定 XDG 路径时使用 `~/.config`；
macOS 按 Zed 的路径使用 `~/.config/zed`，Windows 使用 `%APPDATA%\zed`。
参见 [Zed 设置路径](https://github.com/zed-industries/zed/blob/main/docs/src/configuring-zed.md)。
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
BasG 只读取桌面主题，不修改 Zed 或 DMS 配置。实现参考
[DMS 应用主题](https://danklinux.com/docs/dankmaterialshell/application-themes)、
[Zed workspace / thread 侧栏](https://github.com/zed-industries/zed/blob/main/crates/sidebar/src/sidebar.rs)
及 [Agent thread 视图](https://github.com/zed-industries/zed/blob/main/crates/agent_ui/src/conversation_view/thread_view.rs)。

### 统一设置中的外观偏好

Bas 偏好和 Agent 配置保存在系统用户配置目录下的 `bas/settings.json`；
也可用 `--settings-file /path/to/settings.json` 指定位置。

| 平台 | 默认配置文件 | 默认状态根目录 |
| --- | --- | --- |
| Linux | `~/.config/bas/settings.json` | `~/.local/state` |
| macOS | `~/Library/Application Support/bas/settings.json` | `~/Library/Application Support` |
| Windows | `%APPDATA%\bas\settings.json` | `%LOCALAPPDATA%` |

绝对的 `XDG_CONFIG_HOME` / `XDG_STATE_HOME` / `XDG_CACHE_HOME` 可覆盖对应目录；
空值和相对路径忽略，不将默认存储重定位到启动目录。daemon 数据库位于状态根目录的
`bas/sessions.sqlite3`，窗口恢复与置顶状态位于 `basg/`。已有 macOS 用户若使用原 Linux
式目录，可以继续通过 XDG 覆盖或显式文件路径使用原数据；不会自动迁移或删除旧文件。
文件不存在时读取旧格式作为迁移来源；没有旧文件则沿用默认，不创建空文件。可在独立设置窗口修改，也可直接编辑 JSON；
两种入口共用校验、合并及热更新规则。
这是严格 JSON：未知字段、非法枚举、空字体名、非正数或非有限字号/行高，以及不在 `[0, 1]`
内的不透明度都会被拒绝。外部 Zed/DMS 文件仍兼容 JSONC 和未知字段。

示例（只填写需要覆盖的项目）：

```json
{
  "appearance": {
    "language": "zh-CN",
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
| `language` | 默认 `zh-CN`；也支持 `en-US`，即时切换界面语言 |
| `ui_font_family` / `mono_font_family` | Zed UI / buffer 字体，然后 DMS UI / mono 字体，然后运行时平台 fallback |
| `font_fallbacks` | 导入 Zed `ui_font_fallbacks`；显式数组替换导入链，`[]` 清空导入链 |
| `ui_font_size` | 导入 Zed `ui_font_size`，否则 15px |
| `content_font_size` | 导入 Zed `agent_ui_font_size`，否则最终 UI 字号 × 87.5% 取整（最小 1px） |
| `content_line_height` | 1.6 倍正文行高 |
| `content_opacity` | 0.94；只影响正文背景，不降低文字不透明度 |
| `composer_enter_behavior` | 默认 `send`：Enter 发送；`newline`：Enter 换行、Ctrl+Enter 发送（macOS 也支持 Cmd+Enter） |
| `confirm_session_delete` | 默认 `true`；`false` 跳过会话及 Agent 历史的删除确认；设置中对应“删除会话时不再提醒”的反向选项 |
| `default_agent` | 可选 Agent ID；新会话优先使用它，未指定时沿用上一次使用的 Agent；在“设置 → 会话”选择 |
| `background` | 默认 `blurred`；`auto` 沿用外部主题背景策略，或 `opaque` / `transparent` / `blurred` |
| `language` | `zh-CN` 简体中文（默认），`en-US` 英文；保存后立即切换 |
| `theme_source` | 默认 `desktop`；`builtin` 完全不读取桌面主题和字体，使用内置主题再叠加显式偏好 |
| `theme_mode` | 默认 `desktop` 尊重 Zed 选择；`system` 跟随窗口系统模式，`light` / `dark` 固定模式 |
| `theme_file` | 外部 Zed 主题文件路径；相对路径以偏好文件所在目录为基准 |

合并顺序为 **内置默认 → 外部 Zed/DMS 适配 → Bas 显式偏好**。
`--theme-file` 优先于 JSON 的 `theme_file`，仅覆盖外部主题路径，不覆盖其它偏好；
`theme_source: "builtin"` 时外部路径均不使用。CLI 相对路径以启动工作目录为基准。
固定模式覆盖 Zed 的固定主题名/模式，但继续采用对应模式的命名变体和主题颜色覆盖。
使用内置源时，`desktop` 模式等同 `system`。

偏好、选中的外部来源以及系统深浅模式每秒检查一次。每次先读取、解析、校验并完成最终
resolve，全部成功才应用。热更新的 IO、JSON 或校验失败会记录带路径的日志，并保留
**整份上次有效配置**；相同连续错误不重复刷日志。文件删除按缺省/导入重新计算；
权限错误等其它 IO 错误不当作文件缺失。首次启动没有有效配置时使用内置默认。
仍选择 `desktop` 就表示依赖该组导入，坏的桌面配置会拒绝整次更新；
选择 `builtin` 后，坏的桌面文件不会阻止完全显式的 Bas 偏好生效。
建议通过临时文件加原子替换更新配置，避免写入期间的中间状态。

字体存在性检测、去除未安装字体、平台 UI 字体替代，以及末尾追加一个已安装的 CJK
安全回退字体属于 **runtime resolution**，不写回偏好文件，也不是个人桌面的固定路径配置。
因此 `font_fallbacks: []` 清除的是用户/导入链，平台安全回退仍保留。
实际透明与模糊效果取决于窗口合成器；`opaque` 配合 `content_opacity: 1` 可请求不透明外观。
配置模型支持 serde 读写，后续页面可复用同一模型和校验，不需要直接修改外部 JSON。

## 会话管理

侧栏的多对话气泡图标或 `Ctrl+M` 打开独立的会话管理窗口，不替换聊天区域，
也不强制展开最小输入弹窗。每个 daemon 地址复用一个历史窗口；窗口拥有自己的连接，
关闭主窗口后仍可浏览和导入，关闭历史窗口不停止现有对话。

左侧以 **Agent** 分类筛选，可选择单个 Agent 或“全部”，使用相同的左对齐按钮；
选择全部时，结果仍按 Agent ID 分组，同名原始会话 ID 不会跨 Agent 混合。
主工作区筛选默认使用当前会话的 `cwd`，也可选择其他目录或无文件夹图标的“全部工作区”。
筛选按完整主目录精确匹配并传给 ACP `session/list`，不把附加工作根算作主工作区。
标题、工作目录和原始会话 ID 仍可通过搜索框进一步过滤。
卡片不展示 ID，无标题时显示“未命名会话”。数量紧随右侧分组的 Agent 名称；
目录位于卡片左下，所有操作位于右下同一行。已打开记录显示运行状态及独立的归档状态。

时间与会话侧栏、连接检查记录共用相对时间规则，优先显示“刚刚／几分钟前／几小时前”，
每分钟刷新；悬停可查看 Agent 原始时间，缺失或无效时间显示“时间未知”，不伪造最近活动。
导入、打开、刷新、停止和删除操作配有图标；停止直接执行，删除遵循确认偏好。
所有历史操作均为同规格的纯图标按钮，名称通过悬浮提示及无障碍标签提供。
卡片的会话状态复用主窗口侧栏的状态指示组件，不另外显示“执行中／已停止”等文字；
归档独立显示为被动归档图标，悬浮说明状态，不与可点击的恢复操作混用。
浏览不会自动导入、创建会话或发送 prompt，当前聊天草稿保持不变。

- “导入并打开”通过 `session/load` 恢复原会话，并将旧消息回放保存到 Bas；
  仅支持 `session/resume` 时显示“续接（无历史回放）”，不会假造旧消息。
- 只声明 list、没有加载或恢复能力的 Agent 可以浏览但不能导入。
  重复导入同一 Agent／会话 ID 返回原 Bas 记录；默认 localdb 保留映射和回放。
- Agent 返回的工作目录按原样使用，不改写符号链接路径。包含额外工作根的会话暂不支持导入，
  会明确提示，不能静默丢掉这些根。
- 历史目录和 thread 右键菜单的“停止会话”直接停止运行实例。
  支持时先调用标准 ACP close，否则释放本地实例；保留会话和历史，不删除 Agent 历史。
- 每条会话只有一个主要入口：未归档的普通会话为“打开会话”（单对话气泡），
  已停止的为“重新启动并打开”（播放三角），已归档的为“恢复会话”（取消归档图标）。
  状态变化后旧按钮回调失效，不会将原先的打开操作隐式改为启动或恢复。
- “重新启动并打开”显式重新加载停止会话的原 Agent 上下文，
  保持会话 ID 和历史，不自动发送消息或回退到新建会话。ACP Agent 必须支持 load/resume。
  “恢复会话”会启动原会话再取消归档；失败仍保留归档的停止记录。
  停止操作统一称为“停止会话”，使用停止方块，与启动三角对应。
- “删除 Agent 历史”默认单独确认，只调用标准 `session/delete`，保留 Bas 本地记录。
  已导入且活跃的会话必须先关闭；原有本地删除和归档行为不变。
- Agent 未声明的操作不会调用。修改操作不自动重试；错误或超时需刷新列表确认真实状态。
  daemon 和 GUI 都需要更新，旧 daemon 返回不支持时不会退回创建新会话冒充导入。

已停止会话的右键菜单提供“启动会话”，沿用原会话身份；后台会话启动成功不抢走当前选择，
当前会话则重新连接消息流并保留草稿。Agent 不支持加载／恢复时禁用启动。
会话侧栏的运行状态：启动中／运行中为转圈，就绪且已读时留空，失败为警告图标，已停止为灰色熄灭灯；
有未读回复的空闲会话显示小圆点。未读是阅读状态，归档是列表属性；归档必然停止，停止不必然归档。
Agent 图标表示来源，行背景表示选中；等待审批或输入目前没有独立的侧栏图形标记。

## ACP 展示元素

2026-10-10 对照 Zed `main` 的固定提交
[`f16f9652`](https://github.com/zed-industries/zed/tree/f16f9652ec57bf806e65b2a0d51bb92a63644914)。
SDK 升为 3.3.0，启用 `schemars`、`unstable` 和 `unstable_protocol_v2`，
并保留 Bas 子进程传输所需的 `process`。外部 Agent 仍协商 v1；
计划、用量、通知与压缩使用 v2 展示类型，工具和普通消息继续兼容 v1 历史。
这里的“已支持”指当前 Agent→daemon→RPC→GUI 路径可展示；
“部分支持”指已有对应 UI，但 v2 草案新增的事件或结构尚未通过该路径接入。

| 元素 | 当前接入 | BasG 展示与边界 |
| --- | --- | --- |
| 工具调用 | 部分支持：v1 `tool_call` / `tool_call_update` | 名称、标题、状态、输入输出、富内容、文件位置及局部更新；原生 v2 `tool_call_content_chunk` 尚未接入 |
| 文件变更 | 部分支持：v1 `type: "diff"` | 文件路径；提供 old/new 时生成真实 unified patch，无 oldText 时展示修改后全文；尚无 v2 新增／删除／重命名等多文件操作清单，也无 diff 审批 |
| 终端 | 部分支持：Zed 的 `_meta.terminal_info` / `terminal_output` / `terminal_exit` 扩展 | 工具标题或扩展提供的命令、工作目录、ANSI 输出、退出码和信号，空输出退出可见，rawOutput 保留兜底；v2 终端展示 reducer 已实现，原生事件尚未接入，也未声明客户端托管终端执行能力 |
| 计划 | 已支持：v1 `plan`、实验 `plan_update` / `plan_removed` | 条目进度、状态、优先级；按 ID 更新／移除多个计划；Markdown 计划预览；文件计划展示并打开 URI，不读取文件内容 |
| 前台状态 | 部分支持：Bas 回合／生命周期事件、审批和交互订阅 | 执行中、空闲、等待工具审批、等待用户交互；请求在回合外出现时也显示等待状态；v2 `state_update` reducer 已实现，原生事件尚未接入 |
| 工具审批 | 已支持：v1 `session/request_permission` | 独立卡片展示 Agent 提供的允许／拒绝选项，返回原生 option ID；支持用户取消及 Agent 取消请求；不主动拦截 Agent 自行执行的工具 |
| 用户选择／输入 | 已支持：v1 `elicitation/create` / `elicitation/complete` | 表单中的单选、多选、文本、数字和布尔值，提交前校验；URL 交互需用户点击，接受后等待完成通知；紧凑模式自动展开交互区域；需要适配器发出请求 |
| 可用命令 | 已支持：`available_commands_update` | Slash commands 列表、描述、过滤和键盘补全；目录保存在会话快照，不受历史裁剪影响 |
| 会话配置 | 已支持：`config_option_update` | Select 和 Bool 控件及当前值、更新请求与错误；分组选项摊平显示，未知控件类型不提供编辑器 |
| 会话信息 | 已支持：`session_info_update` | 标题与 Agent 的 RFC3339 更新时间；单独保存 Agent 时间，后续 daemon 事件不覆盖；省略保持、Null 清空，无效时间保留上次值 |
| 使用量 | 已支持：`usage_update` | 收到数据后显示输入区工具栏 16px 环形指示器，达到 85% 使用警告色；悬停显示百分比和简写用量／容量；点击展开精确用量、窗口容量、剩余词元 和可选累计费用／币种，打开期间随事件更新 |
| 即时通知 | 已支持：标准 `notice` | 信息／警告／错误 banner、标题与可选说明，可关闭；仅实时接收，不写历史，断线或切换会话后不重放 |
| 上下文压缩 | 已支持：`compaction_update` / `compaction_summary_chunk` | 按 ID 原位更新状态、错误和可展开富内容摘要；支持增量、整体替换、Null 清空和历史回放 |

这 13 类均有展示入口，其中 9 类已接入 v1 事件、请求或实验扩展，4 类通过兼容路径部分支持。
启用 v2 schema 不等于原生 v2 传输已完成。初始化声明计划、通知和压缩能力，
更新 daemon 后需新建 Agent 会话才能重新协商。
实现与回归记录见 [协议基线](upstream-ui.md#acp-展示协议基线2026-10-10)，
事件持久化和通知游标约定见 [RPC 边界](rpc.md#回放与输入边界)。

### 前台交互与后台任务

工具审批与用户选择使用不同的 ACP 请求。审批返回允许／拒绝选项的 ID；
[Elicitation](https://agentclientprotocol.com/protocol/v1/elicitation) 返回结构化表单答案，
不会将普通选择转换成工具授权。两者都依赖 Agent 或适配器主动发起请求；
只在 Markdown 中列出选项，客户端不会自动生成选择控件。
已核对的 `pi-acp 0.0.34` 会在 Pi 扩展调用 `ctx.ui.select()` 或 `ctx.ui.confirm()` 时
发出 ACP `session/request_permission`，由 BasG 显示审批卡片并返回所选选项；
这种选择不是标准 `elicitation/create` 表单。
Pi 内置工具不会自动调用这两个方法；助手只在正文里列出 1／2／3，也不会生成请求。
该版本未将扩展的 `input`／`editor` 转为 ACP `elicitation/create`。
转换路径见 [pi-acp 0.0.34 源码](https://github.com/svkozak/pi-acp/blob/v0.0.34/src/acp/session.ts#L915)，
触发审批的 Pi 扩展示例见 [permission-gate](https://github.com/earendil-works/pi/blob/v1.1.0/packages/coding-agent/examples/extensions/permission-gate.ts)。

`codex-acp 2.1.1` 能将同步 `item/tool/requestUserInput` 转为表单，
但会丢弃 Codex 异步 `request_user_input_async` 生成的 AgentMessage `questions`。
这类问题在 Codex 中已接受，ACP 客户端却收不到 `elicitation/create`；
仅升级 Bas 或切换 Plan 模式不能补齐这条适配链路。
这属于适配器侧兼容限制，等待上游修复。Bas 已验证标准表单请求在回合结束后
仍可保留、提交及接收后续通知的生命周期；需要适配器实际发出该请求。

Codex 的权限 **Mode** 与 **Collaboration mode** 是独立配置。
`codex-acp 2.1.1` 默认权限预设为 Auto review，会显式传入 `approvals_reviewer=auto_review`；
Default／Plan 的切换不改变审批人。人工审批应选择 Workspace access 或 Read-only，
默认值配置见 [Codex 安装说明](install.md#codex)。恢复会话时，Bas 将后端返回的
当前配置保存并通知 GUI，避免保留恢复前的权限显示；适配器仍决定恢复后的实际值。

2026-10-11 对照 Zed `main`
[`96984225`](https://github.com/zed-industries/zed/tree/96984225abc235519e66b5c6c9ed23b87621f868) 的
[ACP handlers](https://github.com/zed-industries/zed/blob/96984225abc235519e66b5c6c9ed23b87621f868/crates/agent_servers/src/acp.rs)
与 [Elicitation UI](https://github.com/zed-industries/zed/blob/96984225abc235519e66b5c6c9ed23b87621f868/crates/agent_ui/src/conversation_view/elicitation.rs)，
补齐交互等待状态及 Agent 取消审批请求的生命周期处理。

ACP 能表达后台工作。Bas 已接入基础工作状态浮层，但尚未提供独立任务管理 RPC：

- **多个会话并行**：[ACP 架构](https://agentclientprotocol.com/get-started/architecture) 允许一条连接承载多个并发会话。
  Bas 当前为各会话维护独立后端与事件流，切换或关闭 GUI 不取消已提交任务。
  这不等于同一会话的多个 prompt 可以无条件并发：Bas 对一个会话只允许一个在途回合，其余输入由 GUI 排队。
- **长终端任务**：v1 [终端接口](https://agentclientprotocol.com/protocol/v1/terminals)
  的 `terminal/create` 立即返回 `terminalId`，后续用 `output`、`wait_for_exit`、`kill`、
  `release` 查询输出、等待、停止和释放。Bas 当前声明 `terminal: false`，
  仅展示 Agent 自行执行后通过工具更新发送的终端内容，尚不托管这些进程。
- **Subagent**：上游 [Subagents RFD](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/rfds/subagents.mdx)
  定义子会话关联与独立事件。Bas 现声明 `clientCapabilities.subagents: {}`，
  接收根及嵌套父会话上的 `subagent_update`，保留名称、描述、状态与每个子会话的能力。
  仅接收已公告子会话的独立事件；子消息、工具、计划等以原 ACP sessionId 保存和传输，
  不并入主回答。子会话请求沿所属主会话的权限／交互通道处理，未知会话 ID 被拒绝。
  浮层显示活动子 Agent 的名称、描述和状态。尚无单独展开子会话全文、向子会话发消息或取消的 GUI；
  不会因为父会话支持某种操作就擅自对孩子执行。
  已核对发布包 `@agentclientprotocol/claude-agent-acp@0.93.0`：
  [适配器说明](https://github.com/agentclientprotocol/claude-agent-acp/blob/dbf386d8d35d2bc30f17f11f15d1a4305c9d391f/README.md#subagent-sessions)
  与包内 `dist/native-subagents.js` 均包含标准 `subagent_update`／子会话事件及工作状态。
  但 v1 客户端必须先声明 `clientCapabilities.subagents`，否则它也会退回普通工具调用。
  该适配器可作为协议级互操作测试对象。声明发生在 daemon 的 ACP 握手中；
  必须更新 daemon 与 GUI，并新建或重新加载会话，旧连接不会自动重新协商。
  目前验证使用受控 ACP 对端，尚未进行真实 Claude 账号的互操作验收。
- **前台空闲时继续工作**：v1 会话通知可在回合外到达，Bas 会作为会话事件接收；
  v2 [前台状态](https://agentclientprotocol.com/protocol/v2/prompt-lifecycle)
  明确区分前台回合与后台工作，`idle` 不表示所有后台任务已结束。
  后续接入应分别保存前台状态和任务状态，并为任务保留 ID、进度、输出及取消操作。
  [v2 草案公告](https://agentclientprotocol.com/announcements/acp-v2-draft) 将 prompt 响应改为输入被接受的确认，
  通过独立状态更新表示前台工作结束，更适合后台完成后主动继续工作、排队和 steering。
  这不是通用任务调度器，也不能据此假设 v1 Agent 已支持；Bas 当前连接仍协商 v1。

## 数据与验证

历史按游标分页读取后接实时订阅，断线自动续接，游标过期时重新加载保留窗口。
切换 thread 使用订阅代次隔离迟到消息；较晚完成的初始化请求不会自动发送消息，
也不会抢走用户正在编辑的 workspace。命令目录保存在原生 ACP 会话快照中，不随历史裁剪丢失。修改请求不会自动重试，避免重复执行。

bas 默认使用 `localdb` 保存会话快照与有序事件，GUI 通过历史分页恢复内容。
`memory` 模式只保留有界内存历史；GUI 不承诺恢复该模式下已淘汰的消息或服务重启前的数据。
Agent 提供 `configOptions` 时，输入区展示选择或开关控件，可用于模型、模式、推理级别等设置；选项顺序遵循 Agent 定义。
对话可展示 ACP 图片、嵌入文本文件、资源链接、工具内容和 diff 文本。
标准 ACP 工具权限请求显示独立审批卡片，保留工具名称、输入、路径与 Agent 提供的
允许／拒绝选项；选择严格返回原生 option ID，另可取消请求。紧凑模式收到请求时展开
输出面板，按钮提交后禁用直到服务端移除请求；错误保留卡片并允许重试。
审批订阅覆盖当前 thread，切换 thread 释放当前订阅；只有最后一个审批客户端离开才
取消该会话尚未回答的请求。进入过交互审批的会话在没有审批客户端时取消后续请求，
不回退为自动允许。
从未进入交互审批的无界面会话沿用 daemon 原有权限策略。旧 daemon 会明确提示升级。
只有后端主动发起标准 ACP 权限请求的工具才经过此界面，工具自己的执行机制仍由后端决定。
标准 `elicitation/create` 表单提供单选、多选和自由输入，答案通过独立 RPC 返回，
保留聊天草稿；请求到达后显示“等待用户交互…”，紧凑模式自动展开。
Agent 取消、回合取消、会话切换或最后一个交互客户端断开时，未回答请求按连接生命周期清理。
音频文件按 Agent 声明的输入能力发送为原生 AudioContent；返回音频提供播放／暂停／停止控件，
历史回放不自动播放，切换会话或关窗会停止。Agent 卡片提供带确认的标准退出登录，
能力不支持时明确提示，不自行删除凭据。能力核对和范围见 [工具扩展、退出登录与音频](acp-tools-and-audio.md)。
目前还没有 diff 审批；Mermaid、TeX 和 Typst 保留为后续功能。
RPC 已支持 ACP 的完整内容类型。

```sh
nix develop .#gui -c cargo check --features gui --bin basg
nix develop .#gui -c cargo test --features gui-tests --bin basg
nix develop -c cargo test --all-targets
```

原生 Wayland MIME 协商回归使用独立的 headless Sway，会创建并清理临时 runtime
目录，不读写当前桌面的剪贴板：

```sh
nix develop .#gui -c cargo build --features gui-tests --bin basg --locked
nix develop .#gui -c nix shell --inputs-from . nixpkgs#sway-unwrapped nixpkgs#python3 -c python3 tests/wayland_clipboard.py
```

测试覆盖分组、两种模式切换、输出折叠、提前初始化、草稿隔离、中文与多行输入、
命令补全、Markdown 流式解析、工具与消息顺序、终端流与结果兜底、
展开消息编辑器、历史裁剪后的命令目录保留、
工具栏与内容宽度、折叠状态进度、剪贴板图片和附件删除 / 撤销、文本附件回退、不支持附件的插入拦截与错误清理、
附件草稿与发送失败、归档与恢复、主题选择与透明度，
连接及会话初始化期间的发送拦截，
以及真实本机 gRPC 的附件传输、历史与断线恢复。
无界面测试不替代实际桌面上的输入法、缩放、透明合成和焦点验证。

可用独立的固定内容预览核对桌面视觉，不连接 bas 或发起 Agent 请求：

```sh
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel empty
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel sample
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel layout
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel connecting
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel disconnected
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel menus
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel files
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel thoughts
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel tool-details
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel turn-navigation --window
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel turn-navigation-hover --window
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel turn-navigation-many
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel completed
nix develop .#gui -c cargo run --features gui-tests --bin basg -- --preview-panel reply-preview
# 加 --window 查看完整窗口布局
```
