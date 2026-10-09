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

## 验证与逐项处理

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
