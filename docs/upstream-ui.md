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

根锁文件为三个原本使用 path 覆盖的包补充 registry source 和 checksum，其他版本保持不变。
BASR 第一方代码仍是 MIT，不复制或链接 Zed 的 GPL 上层组件；第三方依赖保留原许可证。
此前的会话、队列、RPC、路径校验、日志和外观配置修复不因这次切换撤销。

仅移除了两处已不存在的私有扩展接入：

- 行内代码不再调用本地新增的字号比例 API，接受上游默认行为。
- 输入框不再调用本地新增的视觉行数 API，暂时用上游公开文本缓冲区 API 按逻辑行数
  （包括末尾换行产生的空行）决定高度。
  没有复制原来的内部实现或增加字符宽度估算来掩盖缺口。

## 已确认问题

### UI-001：输入文字与工具栏左边缘相差 10px

现有测试
`workspace::tests::composer_toolbar_keeps_options_attachments_and_send_on_one_row_in_both_modes`
失败。基线测量中，输入光标左边缘为 `287px`，附件按钮左边缘为 `277px`。
发布版编辑器在当前配置下保留了一段额外 gutter。

原断言保留，不修改 padding 抵消，也不为了测试变绿放宽对齐要求。

### UI-002：长单行文本软换行后，输入框不随之增高

新增回归
`workspace::tests::soft_wrapped_composer_text_increases_the_editor_height`
失败。输入足以软换行的长单行文本后，输入框仍保持初始的 `37px` 高度。
这是移除视觉行数扩展后尚未实现的应用层适配，不表示整个软换行能力不存在。

显式换行计数只用于维持可运行基线，不是最终的自动高度方案。
另有通过的 `explicit_line_break_at_end_increases_the_editor_height` 回归，确保最小接入
没有额外丢失 Shift+Enter 末尾空行的高度。

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
发送按钮在初始化失败后的 tooltip 边界。GUI 全套 171 项通过，UI-001、UI-002
两项既有断言仍失败；严格 Clippy 与所改 Rust 文件格式检查通过。真实桌面合成器
的视觉呈现仍需人工确认，headless 边界测量不等于实机截图。

### 删除后图片编号递增

当前草稿保留已删除附件的内容以支持 Undo，新的附件引用不会复用旧名字，因此删除
Image 后再次插入可能显示 Image 2。这不是当前正在发送的附件数量；发送时只使用
文本中仍存在的完整附件引用。若需要显示序号连续，应将展示序号与稳定引用解耦，
不能简单复用名字而令撤销后的引用指向另一张图片。

## 仍需复核的边界

用户提供的 emoji、中英文混排样例已实测正常，不再将该样例列为故障。
以下是尚未覆盖的额外边界，不能把原补丁说明当作验收结果：

| 项目 | 复核方式 |
| --- | --- |
| 缩放与重新测量 | 已有混排样例正常；可进一步改变字号、行高和窗口宽度，滚出再滚回，检查是否出现新的裁剪、重叠或异常留白 |
| Wayland 多格式剪贴板 | 分别从浏览器、截图工具、文件管理器粘贴；覆盖图片与文本并存、纯图片、文件 URI 和普通文本 |
| Portal 文件选择功能 | UI-006 的 warning 已确认；打开、保存、取消是否存在功能异常仍需分别验证 |

Headless 剪贴板测试直接注入 `ClipboardItem`，不覆盖合成器实际交付哪些 MIME。
移除 vendor 后仍通过这些测试，不等于 Wayland 数据协商已验证。

UI-003、UI-004 是用户实测观察，尚未有对应的自动化复现；不能据现有 GUI 测试通过项
将其判为已经正常。

## 验证与逐项处理

### 设置侧栏文字对齐

发布版 `gpui-component 0.6.1` 的 `Button::label` 使用独立的、占满宽度且居中的内部容器。
仅给按钮根元素 `.justify_start()` 不会改变标签对齐；窄、宽标签都居中即可复现。
设置导航通过公开 `Button::child` 提供可伸展的左对齐文字，并保留 accessibility label；
不修改依赖。回归同时检查类别、子项、动态 Agent 名称的文字起点和按钮语义。

本机 Linux / Nix 基线结果：

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

当前 GUI 基线保留 UI-001、UI-002 的失败断言，完整 GUI 测试与 CI 因而不能记为全绿。
移除覆盖后从公开 API 和应用层适配入手逐个修复，不恢复 vendor
或修改 Cargo registry cache，也不通过忽略测试掩盖差异。
