# 原版 UI 依赖基线

## 依赖与范围

项目不再维护 `vendor/` 源码或 `[patch.crates-io]` 覆盖。依赖版本不借机升级：

| Crate | 版本 | 来源 |
| --- | --- | --- |
| `gpui-kit` | 0.6.1 | crates.io |
| `gpui-base` | 0.6.1 | crates.io |
| `gpui-pre-linux` | 0.3.5 | crates.io |
| `ashpd` | 0.13.13 | crates.io |
| `gpui-pre-reqwest-client` | 0.3.5 | crates.io，GUI 资源 HTTP 适配器 |

移除覆盖时，根锁文件为三个原本使用 path 覆盖的包补充 registry source 和 checksum。
窗口适配随后新增 `wl-clipboard-rs 0.9.4`、macOS 的 `arboard 3.6.1` 与 PNG 编码依赖，
共享用户目录使用 `dirs 6`；这些依赖均为 MIT / Apache-2.0 发布包，不改 GPUI 版本。
BASR 第一方代码仍是 MIT，不复制或链接 Zed 的 GPL 上层组件；第三方依赖保留原许可证。
此前的会话、队列、RPC、路径校验、日志和外观配置修复不因这次切换撤销。

仅移除了两处已不存在的私有扩展接入：

- 行内代码不再调用本地新增的字号比例 API，接受上游默认行为。
- 输入框不再调用本地新增的视觉行数 API；应用通过公开 `DisplayMap` 和字体布局
  测量软换行行数，并保留末尾换行产生的空行。
  没有复制原来的内部实现或增加字符宽度估算来掩盖缺口。

## 已确认问题

### UI-001：输入文字与工具栏左边缘相差 10px

移除覆盖后的测试
`workspace::tests::composer_toolbar_keeps_options_attachments_and_send_on_one_row_in_both_modes`
失败。基线测量中，输入光标左边缘为 `287px`，附件按钮左边缘为 `277px`。
发布版编辑器在当前配置下保留了一段额外 gutter。

现已通过公开 `text_bounds` 与 `EntityInputHandler::bounds_for_range(0..0)` 测量实际文本
起点，将控件左侧 inset 设为组件自身 padding 加测得的 gutter。该逻辑封装为
`input_geometry`，供输入框的附属控件复用，不依赖硬编码的 10px 偏移。
编辑器在 paint 时发布几何信息，附属布局随几何通知在后续帧更新。
原来光标与附件按钮严格相等的断言保留并通过；新增回归覆盖宽度、内容字号和光标移动，
确认按钮跟随文字起点，不跟随当前光标。

### UI-002：长单行文本软换行后，输入框不随之增高

新增回归
`workspace::tests::soft_wrapped_composer_text_increases_the_editor_height`
失败。输入足以软换行的长单行文本后，输入框仍保持初始的 `37px` 高度。
这是移除视觉行数扩展后尚未实现的应用层适配，不表示整个软换行能力不存在。

现已通过应用侧的 `InputLayout` 接入公开 `DisplayMap` 修复。更新使用
`on_text_changed`，`set_text` 只用于初始化，不能当作已初始化布局的全文替换。
宽度来自编辑器实际 `text_bounds`；固定版本的代码编辑器仍为无行号 gutter 和右侧
光标各保留 10px，测量使用相同可用宽度，不修改编辑器或工具栏 padding。
字体与 fallback 使用应用主题，布局改变后随编辑器几何通知重新测量。

原 `soft_wrapped_composer_text_increases_the_editor_height` 断言通过；新增回归覆盖窄／宽／
再变窄、中文与 emoji 混排、字号变化、紧凑模式四行上限和末尾空行。
UI-001 的额外 gutter 左对齐问题由独立的公开 API 几何适配处理，见上文。

## 用户实测观察，待自动化复现

### UI-003：Markdown 列表显示异常

用户在无 vendor 的本地安装版中报告列表渲染问题，截图包含有序列表、列表项内文本
以及代码块。后续保存原始 Markdown，核对缩进、编号与块间距，再建立最小回归；
目前不根据截图推断具体是解析还是布局层导致。

### UI-004：行内代码显示异常

用户在同一安装版中报告行内代码问题。后续核对字号、背景覆盖、基线及换行边界，
不能仅因上游默认字号比例为 0.875 就断定根因。按用户实际期望从应用层和公开 API 处理。

## 附件实测与处理

### UI-005：Ctrl+Backspace 破坏图片占位符

用户拖入图片后按 Ctrl+Backspace，只删掉了占位符的右括号。该现象已在实际拖入事件的
headless 回归中复现。源码已补齐前后两个按词删除 action 的应用层保护：使用上游同一
Unicode 分词库确定候选范围，只在删除范围碰到附件时扩展为整个占位符；普通文本仍由
原生 action 处理。显式范围编辑保留 Undo 前的光标，而不是先制造临时选区再删除。

回归覆盖拖入图片、Ctrl+Backspace / Ctrl+Delete、中文和 emoji 前缀、空白、已有选区、
普通文本以及撤销恢复的文本、附件引用与光标。没有恢复 vendor 或修改依赖缓存。
此修复已包含在本地安装包；已打开的旧进程需要重新启动 GUI 才能使用新版。

### UI-006：文件选择时出现 Portal GetAll warning

用户在添加图片时实际观察到 `zbus::proxy` 的属性缓存 warning：
对 `/org/freedesktop/portal/desktop/request/...` 执行 `GetAll` 返回
`org.freedesktop.DBus.Error.UnknownMethod`，提示 Request 对象不存在。
这项日志问题已确认，尚未处理；目前没有据此确认图片添加失败。
不通过屏蔽整个 zbus 日志目标来假装修复。

### UI-007：远程图片报 No HttpClient available

根因是应用没有为 GPUI 注册 HTTP client，并非图片地址已返回错误。源码已在窗口创建前
接入正式发布的 Apache-2.0 HTTP 适配器，复用 GUI 的 Tokio 网络运行时，不修改 GPUI。
资源请求默认使用 30 秒总期限，覆盖响应体读取，并保留调用方指定的期限。

回归通过真实 GPUI `ImageAssetLoader` 加载 loopback SVG，验证解码、重定向、404 和停滞
响应体超时；GPUI future 从非 Tokio 执行器轮询，防止把运行时桥接错误藏在测试环境中。
没有在测试中请求用户提供的外部站点，也没有关闭 TLS 证书校验。
此修复已包含在本地安装包。

### UI-008：Mermaid 渲染（远期待办）

用户观察到 `mermaid` 未注册、语法高亮回退为 `text` 的 warning。按用户要求保留为远期
功能：当前 Mermaid fence 仍按代码块显示，没有图形渲染。注册一个高亮名称不等于实现
Mermaid 渲染，本轮不通过伪注册语言或屏蔽 warning 宣称完成。

### UI-009：线程右键菜单关闭后保留 Entity

使用发布版 `ContextMenuExt` 打开线程菜单并执行操作后，GPUI 回归在销毁窗口时报告
`PopupMenu` Entity 泄漏。其共享状态持有订阅，而订阅闭包又持有共享状态；关闭菜单
没有释放这条持有链。

应用不修改依赖，线程右键菜单复用 Workspace 已有的菜单所有权与关闭路径：
支持时使用原生 popup，否则使用窗口内 overlay。回归执行置顶、重命名、归档和
删除确认后销毁窗口，验证操作目标与生命周期；不关闭 GPUI 的泄漏检测。

### UI-010：紧凑窗口的动态 tooltip 被窗口边界裁切

复现：以最小宽度 420px 的 compact 窗口启动，令 Agent 初始化失败并在发送按钮上
停留至 tooltip 出现。将完整错误信息放在按钮提示中时，发布版 `gpui-component
0.6.1` 的默认单行 tooltip 可以超过窗口宽度；窗口内 overlay 无法显示超出原生
窗口的部分。长路径、无空格输出及多行错误也有相同风险。保持窗口内 overlay，
使用公开的 tooltip builder 限定宽高并允许换行/滚动，不改依赖或协议。

应用侧复用窗口尺寸约束的 hoverable tooltip：长 token 按 grapheme 插入零宽换行点，
内容区域可滚动，弹层仍在原生窗口内。回归通过实际鼠标移动及显示延时，测量
420×80、420×240、1120×800 三种窗口的长错误、多行提示，并直接检查 compact
发送按钮在初始化失败后的 tooltip 边界。当时 GUI 全套 171 项通过，UI-001、UI-002
两项既有断言仍失败；这两项现已由后续应用适配修复。真实桌面合成器
的视觉呈现仍需人工确认，headless 边界测量不等于实机截图。

### UI-011：Markdown 有序列表丢失非 1 起始编号

发布版 `gpui-base 0.6.1` 的 Markdown AST 到节点转换未保存 `mdast::List.start`，
渲染列表又从零开始计数。最小复现为 `3. third\n4. fourth`，显示编号成为 1、2。
多级有序列表还使用组件自己的字母编号策略。当前整表 Markdown 复制、多级列表、
列表内代码与表格回归不表示该编号问题已修复，也不能据此关闭 UI-003 的用户报告。
保留成熟的上游解析与布局实现，编号修复需要上游公开节点／渲染 API 支持。

### 删除后图片编号递增

当前草稿保留已删除附件的内容以支持 Undo，新的附件引用不会复用旧名字，因此删除
Image 后再次插入可能显示 Image 2。这不是当前正在发送的附件数量；发送时只使用
文本中仍存在的完整附件引用。若需要显示序号连续，应将展示序号与稳定引用解耦，
不能简单复用名字而令撤销后的引用指向另一张图片。

### UI-012：原生剪贴板只读取第一个匹配格式

发布版 `gpui-pre-linux 0.3.5` 的 Wayland 读取优先返回 UTF-8 文本，图片只在文本不存在
时读取；没有处理文件管理器的 `text/uri-list`，也未将纯 `text/plain` 作为文本回退。
macOS pasteboard 同样优先文本。直接在 headless 注入多项 `ClipboardItem` 无法复现
平台层丢格式的问题。

应用用发布版 [wl-clipboard-rs](https://docs.rs/wl-clipboard-rs/0.9.4/wl_clipboard_rs/paste/index.html)
与 macOS [arboard](https://docs.rs/arboard/3.6.1/arboard/struct.Clipboard.html) 补充原生读取，
保留同时存在的文字和图片，读取本机文件 URI；Windows 保留 GPUI 原有多格式路径。
读取放在后台，有三秒应用期限和单项读取大小限制，不阻塞输入 UI；期间输入和发送禁用。
草稿代次与内容快照共同防止 thread 切换、队列编辑或程序修改草稿后的迟到结果混入。
合成器没有 data-control 或读取失败时回退到已有 GPUI 内容，不宣称回退仍覆盖所有 MIME。

`tests/wayland_clipboard.py` 建立独立 headless Sway，实际提供纯 `text/plain`、
文本与 PNG 并存、文件 URI 三种 offer 并读取验收；不会覆盖用户当前桌面的剪贴板。
headless Workspace 回归检查发送／输入拦截、失败回退与 Undo、切换后丢弃、首次初始化
保留粘贴，以及图片不受支持时仍保留文字。macOS 实机 pasteboard 与 Windows 外部来源
粘贴仍需对应平台桌面验收。

### UI-013：ACP Registry 的 Download 与 Scale 图标未打包

实机 ACP Registry 显示安装和许可证按钮时，日志报告
`could not find asset 'icons/download.svg'` 与 `icons/scale.svg`。
发布版 `IconName` 包含完整名称，但默认 `Assets` 只打包组件所需的子集。
应用通过公开 `icon_assets!` 宏显式嵌入 Download、Scale 和 Bot，与既有资产源组合；
不升级依赖、不修改缓存、不嵌入整个图标目录。回归加载目录实际使用的图标，并用
GPUI 自身的 SVG renderer 确认可解析且存在可见像素。

深色目录曾对远端 SVG 使用首字母 avatar；这会丢失后端的品牌标识，见 UI-014。

### UI-014：ACP 目录以首字母替代真实图标，thread 缺少后端标识

用户实机截图中 Agoragentic、Amp 和 Auggie 均显示字母 A，侧栏 thread 标题前没有
对应后端图标。官方 Registry 已提供 SVG 标识；直接用 `img` 保留其固定黑色填充会在
深色背景上消失，以字母替换也无法表达后端身份。需要通过公开 SVG API 按主题绘制
真实轮廓，并为既有 thread 按会话自身的 Agent ID 取得图标，而非使用当前新建会话的
选择。目录、侧栏与选择器复用同一图标路径；缺失或加载失败时才显示通用标识。

应用侧现通过共享 `AgentIcon` 取得原始图标字节，先用公开 `SvgRenderer::parse_svg`
校验，再以 `svg().data()` 的 alpha mask 按主题文字色绘制，不修改远端 SVG。
PNG 等位图保留原色。请求按 URI 共用缓存，包含失败结果，并限制响应体大小和总耗时。
共享目录元数据按 daemon 地址隔离，支持将自定义 `pi` 的 `pi-acp` 启动器关联到官方标识。
侧栏按 `session.agent` 取图标，新 thread 草稿和选择按钮按实际选中的后端显示。
回归覆盖真实 SVG 与位图、深浅主题、失败回退、内置资源、身份匹配和行内布局稳定性。

### UI-015：已有会话的 Agent 按钮仍能打开选择菜单

用户实机点击已选定后端的会话底部 Agent 图标，仍可打开菜单。当前按钮为了保留
安装入口始终可点击，空历史的已创建会话也允许更换后端。会话一旦创建，其 Agent
身份应固定；按钮应禁用，并拦截创建期间或旧菜单迟到的切换操作。未创建的草稿
继续提供选择和安装入口；显式新建 thread 先保留草稿，选择 Agent 后再初始化，
避免目录轮询自动创建并提前锁住选择。

发布版 Button 的 `disabled` 会阻止 pointer 操作，但没有在测试的 AccessKit 节点中
写入 disabled 属性；因此用实际点击、菜单是否打开和是否发送 RPC 验证禁用行为，
不以不存在的 snapshot 属性替代交互验证。

现已在已有会话及创建期间禁用按钮，并在内部处理选择的入口添加同样的守卫。
显式新 thread 的选择草稿不受目录刷新或 F5 自动创建；目录“开始对话”仍能创建
独立的新会话并保留旧草稿。创建和等待认证期间，旧目录快照不会覆盖已选 Agent。

### UI-016：Fast mode 使用泛用齿轮，检测和登录状态含糊

用户实机看到 Codex Fast mode 以齿轮表示，缺少速度模式的语义提示。目录同时显示
“已检测到 · 待连接”和“连接 / 登录”，把程序可用性、一次性的协议检测和认证混为
一项持续连接状态。用户随后补充 Fast mode 的“名称 + 滑动开关”参考图：所有 ACP
Boolean 配置项应显示名称与原版 Switch，直接表达当前开关状态，无需推断后端名称
或配置 ID 的图标含义。目录分别显示安装、检测结果与登录需求，并提供独立的检测
和认证操作。

Boolean 控件现使用公开 Switch API，按后端确认的状态绘制；请求期间禁用，配置后
恢复输入焦点。名称和开关共用受窗口约束的说明 tooltip。目录的“检测 / 重新检测”
仅执行一次校验，检测结果注明其历史含义；登录仅在后端返回认证需求时单独提供，
“开始对话”是另一项操作。回归检查实际点击后的配置值、窄窗布局和检测/登录的
RPC 边界，不调用真实账户的模型请求。

### UI-017：Kimi 登录终端打开后立即退出

用户在已安装的 Kimi 条目中选择终端登录，在确认窗口点击“打开登录终端”，
新终端窗口立即退出，无法完成登录。确认窗口显示的程序来自托管 Kimi 1.52.0，
参数为 `["acp", "login"]`。需要核对认证命令与终端助手的启动结果，并让失败原因
留在终端中供用户查看；不得把终端成功打开当作登录成功。

实际运行旧参数得到退出码 2 和 `Got unexpected extra argument (login)`。
[Kimi 1.52.0 的认证描述](https://github.com/MoonshotAI/kimi-cli/blob/1.52.0/src/kimi_cli/acp/server.py)
返回登录参数 `login`，而该版本的 `login` 与 `acp` 是同级子命令。
初始化返回的 `_meta["terminal-auth"]` 描述完整命令，错误中的同 ID 展开方法
仅在参数、环境一致时采用该执行规则；标准 Terminal 继续遵循
[ACP 的参数追加规则](https://agentclientprotocol.com/protocol/v1/authentication)。
执行计划由 daemon 对照托管启动记录验证，程序使用经过检测的本机路径，环境按描述合并。

登录助手现在将失败退出码及时反馈给 GUI，并在终端保留实际错误，等待 Enter 关闭。
启动登录程序失败同样保留系统错误。终端提前退出且未能给出登录结果时及时提示重开，
不再一直等待登录超时。结果文件采用原子发布，读取中的空文件不被当作登录失败。
回归覆盖真实进程的参数边界、失败退出码、启动失败、终端保持打开时的反馈和提前退出。

### UI-018：检测提示描述内部细节和未执行的操作

用户在 Agent 设置页点击“检测”，页面显示“正在执行一次 ACP 检测；不发送聊天消息，
也不创建 BASR thread。”用户无法从中理解当前动作。检测提示应直接说明正在检查
程序能否正常启动、是否需要登录；完成后显示上次检测结果与下一步操作。程序路径、
协议调用和请求边界保留在配置与诊断说明中，正常状态提示采用面向用户的表述。

现已将进行中的提示改为“正在检查 Agent 是否能正常启动、是否需要登录。”完成后的
结果保留检测时间，并明确提示登录或开始对话。程序已找到、检查程序、等待登录和
尚未检测等状态也统一描述当前动作或下一步操作。

## 仍需复核的边界

用户提供的 emoji、中英文混排样例已实测正常，不再将该样例列为故障。
以下是尚未覆盖的额外边界，不能把原补丁说明当作验收结果：

| 项目 | 复核方式 |
| --- | --- |
| 缩放与重新测量 | 已有混排样例正常；可进一步改变字号、行高和窗口宽度，滚出再滚回，检查是否出现新的裁剪、重叠或异常留白 |
| Wayland 外部应用剪贴板 | 独立 Sway 已验证真实 MIME 协商；继续从浏览器、截图工具、文件管理器检查实际应用 offer 与回退合成器 |
| Portal 文件选择功能 | UI-006 的 warning 已确认；打开、保存、取消是否存在功能异常仍需分别验证 |

Headless 剪贴板测试直接注入 `ClipboardItem`；原生数据协商由独立 Sway 的回归补充。
两者都不替代用户桌面上每个外部应用及合成器的实测。

UI-003、UI-004 是用户实测观察，尚未有对应的自动化复现；不能据现有 GUI 测试通过项
将其判为已经正常。

### UI-019：ACP 认证描述符与交互链路

复现：安装 Qwen Code 0.25.0，在未配置模型提供方时开始会话，认证页把
`Use OpenAI API key` 显示为“登录”；该方法的终端描述符位于 `_meta.type`。
Kimi 1.52.0 的 `_meta["terminal-auth"]` 与错误中展开的方法使用完整应用参数，
按标准 Terminal 的追加参数规则执行会形成无效的 `kimi acp login`。
现有初始化强类型转换丢失未知字段，认证请求缺少双向用户交互通道，无法响应
Agent 的 Form / URL Elicitation。会话内只提供设置页入口，用户需要切换窗口。

处理依据：ACP 原始响应是认证方式的事实来源；标准方法和已验证的历史描述符
分别生成执行计划。设置页与会话页共享认证状态和 Elicitation 组件，保留草稿，
按进程结果与重新检查结果判断完成。参考 Zed v1.23.2 的交互行为，第一方实现
使用公开 ACP / GPUI API，不复制 Zed GPL 组件。回归覆盖原始字段、历史参数、
未知方法、交互取消、URL 完成及迟到回执。

实现使用共享原始 JSON 正规化层和 daemon 双向认证流，标准 Agent／Terminal、完整
历史终端命令及经过验证的 Qwen 0.25.0 应用参数分别处理。未知类型保留描述并禁用，
自定义包装器无法验证启动边界时提示手动配置。设置与会话共用控件，表单校验后提交，
URL 由用户打开并等待同一连接的完成通知。认证成功后保留草稿重试创建会话；已有
会话通过待处理交互快照响应。取消与连接替换清理旧请求，迟到回执按连接代次隔离。

本次 ACP 两阶段验证：默认发布目标 259 项、GUI 发布配置 85 项、GUI headless
275 项全通过。含 GUI 的全目标严格 Clippy、格式与 diff 检查通过，两个 Nix
安装包构建成功。测试涵盖真实 ACP 子进程和 tonic 双向流，但未执行真实账号登录。

## 验证与逐项处理

### UI-020：认证状态与说明层级混杂，终端确认重复，开始失败缺少原因

复现：Gemini CLI 0.63.0 的认证区域中，状态标题、方法按钮与选项说明的字号和颜色
过于接近。点击 Google 登录后显示空的“无法开始认证：”，无法判断是否为 daemon
版本或 Agent 问题。选择 Qwen 终端认证又出现占据大窗的重复确认，增加操作步骤。
处理：明确区分认证状态与辅助说明，合并同一错误的重复展示；点击认证方式直接
执行经过 daemon 验证的流程，终端也直接打开。RPC 错误保留可识别的原因和恢复操作。
回归验证真实按钮点击、旧 daemon 的空 Unimplemented 错误及取消与迟到回执。

现场核对：新版 GUI 连接旧 basrd 时，认证 RPC 返回 `grpc-status: 12` 且没有错误
message。另一个独立缺口是认证流程手写 `session/new` 遗漏必需的 `mcpServers`；
Gemini 0.63.0 严格 schema 会拒绝它。现改用 SDK 构造完整请求，严格 mock 验证认证
前后都带空数组；隔离环境的真实 Gemini 握手确认完整请求能进入认证需求阶段。

本次回归：默认目标 260 项与 GUI headless 278 项全通过，含 GUI 的全目标严格
Clippy、格式和 diff 检查通过。真实按钮经 tonic fixture 验证直接开始终端认证、
取消后恢复可选状态，以及旧 daemon 的空错误转换为明确的重启提示。
隔离 Wayland 窗口确认状态标记与辅助说明的视觉层级。两个 Nix 发布包构建成功，
checkPhase 的默认目标 260 项与 GUI 发布配置 85 项全通过；未执行真实账号登录。

### UI-021：浏览器授权成功后，ACP 认证失败只显示阶段名称

复现：Gemini CLI 0.63.0 打开 Google 授权页，网页显示身份验证成功，BASR 仍显示
“认证失败：Agent authentication failed”。认证 RPC 将带有 context 的错误用
`to_string()` 转为客户端提示，丢失内层 Agent 错误，无法判断 OAuth 回调后的失败阶段。
应保留阶段、ACP 错误码与明确的文字原因，避免将完整错误数据当作提示直接展开。
浏览器由 Agent 的标准 `authenticate` 实现打开，客户端等待同一请求的结果；
网页成功不能替代 Agent 完成认证和后续会话检测。

现场使用当前配置执行只读 `initialize` / `session/new`，会话创建已成功；
未重发认证或聊天请求，也不能据此回溯上一次失败的具体原因。认证 RPC 现保留
阶段、ACP 错误码和单层文字诊断，两种 RPC 共用格式化；认证错误继续作为失败返回。
真实 ACP 子进程与 tonic 回归覆盖认证前后各阶段，以及 SDK 字符串、`message`、
`details` 诊断格式和凭据过滤。默认全目标 267 项测试、严格 Clippy、格式与 diff
检查通过；Nix 后台发布包的 267 项测试同样通过。安装并切换后台后，真实
`basrctl agent check gemini` 返回 `ACP initialize and newSession succeeded`。

### 设置侧栏文字对齐

发布版 `gpui-component 0.6.1` 的 `Button::label` 使用独立的、占满宽度且居中的内部容器。
仅给按钮根元素 `.justify_start()` 不会改变标签对齐；窄、宽标签都居中即可复现。
设置导航通过公开 `Button::child` 提供可伸展的左对齐文字，并保留 accessibility label；
不修改依赖。回归同时检查类别、子项、动态 Agent 名称的文字起点和按钮语义。

移除覆盖时的 Linux / Nix 历史基线结果：

| 检查 | 结果 |
| --- | --- |
| 默认目标测试 | 117 项通过 |
| GUI 测试 | 158 项通过，UI-001、UI-002 两项失败；无忽略项 |
| 含 GUI 的全目标构建 | 通过 |
| 含 GUI 的严格 Clippy | 通过 |
| 格式与 diff 检查 | 通过 |
| Nix 构建与 checkPhase | 通过；daemon 117 项、GUI 包的 `gui` 配置 55 项通过；包含 UI-005、UI-007 修复，未运行系统虚拟机测试 |

```sh
cargo test --all-targets --locked
nix develop .#gui -c cargo build --all-targets --features gui-tests --locked
nix develop .#gui -c cargo test --features gui-tests --bin basrw --locked
nix develop .#gui -c cargo clippy --all-targets --features gui-tests --locked -- -D warnings
```

上述移除覆盖时的 GUI 基线保留 UI-001、UI-002 的失败断言，因而不能记为全绿。
移除覆盖后从公开 API 和应用层适配入手逐个修复，不恢复 vendor
或修改 Cargo registry cache，也不通过忽略测试掩盖差异。

窗口设计的应用侧适配包括软换行高度重测、跨平台窗口按钮、交互审批、队列编辑／撤回
和立即发送，以及思考／工具活动分组、复杂 Markdown 与整表复制。
输入框工具栏不再显示单行／多行开关，Enter 行为在设置中持久化，热更新保留草稿和焦点。
消息编辑器通过 Alt+Shift+Esc 展开／收起，展开时 Enter 换行、Ctrl+Enter 发送。
UI-001 既有左边缘对齐断言通过，没有放宽要求或恢复依赖覆盖。

当前验证命令与原生剪贴板运行器见 [GUI 说明](gui.md#数据与验证)。
CI 在 Linux 跑默认目标、GUI、headless GUI 与独立 Wayland 检查；macOS、Windows
新增原生全目标构建与共享配置、桌面状态测试，本机 Linux 结果不能替代这两个平台验收。

2026-10-09 本机 Linux / Nix 验证：

| 检查 | 结果 |
| --- | --- |
| 默认全目标测试 | 180 项通过（库 34、CLI 15、daemon 117、集成 14） |
| GUI 发布 feature 测试 | 77 项通过 |
| GUI headless 测试 | 232 项通过，无失败或忽略；包含 UI-001、UI-002 原断言 |
| 独立 Wayland 剪贴板 | 纯文本、图片与文本并存、文件 URI 检查通过 |
| 含 GUI 的全目标构建 | 通过 |
| 默认／GUI 全目标严格 Clippy | 通过 |
| Rust 格式与 diff 检查 | 通过 |
| macOS Nix GUI shell | aarch64-darwin 依赖求值通过，未引入 Linux 图形库；没有代替原生构建和实机验收 |
