# 破坏性改名与手动迁移

项目正式名称为 **Bas（Basic Agent Service）**，守护进程为 Bas，图形客户端为
BasG，终端客户端为 BasT。本次是 **breaking rename**：服务与所有客户端必须一起更新，
不支持旧命令、旧环境变量或旧 gRPC 名称，不提供兼容别名，也不会自动迁移用户数据。
仓库地址为 <https://github.com/4t145/bas>。

## 旧名 → 新名对照

本节保留旧名，仅供识别已有安装和手动迁移使用。

| 对象 | 旧名 | 新名 |
| --- | --- | --- |
| 项目 | BASR / Basic Agent Service in Rust | Bas / Basic Agent Service |
| daemon 命令 | `basrd` | `bas` |
| GUI 命令 / 展示名 | `basrw` / BASR | `basg` / BasG |
| 终端命令 / 展示名 | `basrctl` | `bast` / BasT |
| Rust 包与库 | `basr` | `bas` |
| daemon / 共用环境变量前缀 | `BASRD_*`、`BASR_*` | `BAS_*` |
| GUI 专用环境变量前缀 | `BASRW_*` | `BASG_*` |
| 终端专用环境变量前缀 | `BASRCTL_*` | `BAST_*` |
| gRPC package / service | `basr.v1` / `Basr` | `bas.v1` / `Bas` |
| 桌面 app_id | `org.basr.Gui` | `org.bas.Gui` |
| systemd 单元 | `basrd.service` | `bas.service` |
| Nix packages | `basrd`、`basrw` | `bas`（daemon + CLI）、`basg` |
| Nix apps | `basrd`、`basrw` | `bas`、`bast`、`basg` |
| Make 安装目标 | `install-basrd`、`install-basrd-service`、`install-basrw` | `install-bas`、`install-bas-service`、`install-basg` |
| 安装脚本 | `scripts/install-basrd`、`scripts/install-basrw` | `scripts/install-bas`、`scripts/install-basg` |
| 设置文件（配置根目录下） | `basr/settings.json` | `bas/settings.json` |
| 服务环境文件（配置根目录下） | `basr/basrd.env` | `bas/bas.env` |
| daemon 数据库（状态根目录下） | `basr/sessions.sqlite3` | `bas/sessions.sqlite3` |
| 托管 ACP 数据（本地数据根目录下） | `basr/acp/` | `bas/acp/` |
| 托管安装清单文件名 | `basr-installation.json` | `bas-installation.json` |
| 补充信息敏感字段 metadata key | `basr/sensitive` | `bas/sensitive` |
| GUI 状态（状态根目录下） | `basrw/` | `basg/` |
| GitHub 仓库 | `https://github.com/4t145/basr` | `https://github.com/4t145/bas` |

例如 Linux 默认设置文件从 `~/.config/basr/settings.json` 移至
`~/.config/bas/settings.json`；默认数据库从
`~/.local/state/basr/sessions.sqlite3` 移至 `~/.local/state/bas/sessions.sqlite3`；
托管数据从 `~/.local/share/basr/acp/` 移至 `~/.local/share/bas/acp/`；
GUI 状态从 `~/.local/state/basrw/` 移至 `~/.local/state/basg/`。

## 先确认实际路径

上表是相对默认根目录的路径，不能直接用于所有安装：

- 设置根目录由 [共享目录规则](../src/config/paths.rs) 选择：绝对
  `XDG_CONFIG_HOME` 优先，否则使用系统用户配置目录。`--settings-file` /
  `BAS_SETTINGS_FILE` 可覆盖完整设置文件位置。
- 数据库使用同一模块的状态根目录：绝对 `XDG_STATE_HOME` 优先，否则使用系统状态目录，
  不可用时回退至本地数据目录。默认文件为 `bas/sessions.sqlite3`；
  `--localdb-path` / `BAS_LOCALDB_PATH` 可覆盖它，`memory` 模式不持久化。
  路径解析见 [daemon 配置](../src/bin/bas/config/daemon.rs)。
- 托管 ACP 数据使用 `dirs::data_local_dir()` 下的 `bas/acp`，或
  `BAS_ACP_DATA_DIR` 指定的绝对目录，见 [安装管理](../src/bin/bas/management.rs)。
  Linux 通常为 `$XDG_DATA_HOME` 或 `~/.local/share` 下的目录。
- GUI 选择与置顶状态使用状态根目录下的 `basg/`，按服务地址生成 JSON 文件名，
  见 [窗口恢复存储](../src/bin/basg/workspace/navigation/restore.rs)。
  移动目录时保留文件名；这不是会话数据库。

跨平台配置、状态根目录见 [GUI 目录说明](gui.md#统一设置中的外观偏好)。
先核对旧进程的启动参数、服务环境文件、XDG 覆盖和运行用户；自定义目录可能不在默认位置，
也可能与其他应用共享父目录。不要移动整个共享父目录。

## 保守的手动升级顺序

1. **盘点并备份。** 记录当前版本、安装方式、服务单元、启动参数和实际目录，
   备份设置、服务环境文件及外部 Agent 配置。不要把含凭据的备份放进仓库。
   内存模式的会话无法通过复制文件保留；先完成或取消正在执行的任务。
2. **停止旧进程。** 关闭 GUI 和终端订阅，停止并禁用旧服务，确认 daemon 和它的
   Agent 子进程已经退出，避免旧服务与新服务争用端口或数据。
   对停止后的状态、托管数据和 GUI 状态做完整备份；SQLite 数据库及仍存在的
   `-wal`、`-shm` 伴随文件应作为同一份数据保留，不要只复制正在写入的数据库主文件。
3. **手动移动数据。** 按上表把所需的设置文件、服务环境文件、daemon 状态、
   托管 ACP 数据和 GUI 状态移到新位置，保留权限与所有权。仅移动已确认属于该应用的目录。
   若新旧位置都已有内容，先停下并分别备份、逐项核对，不覆盖或盲目合并数据库。
   本次改名不要求转换旧 JSON 格式或改写历史会话，也不要对数据库或历史内容做批量文本替换。
4. **更新引用。** 在 shell、服务环境文件、启动器和自动化中按上表更新命令与环境变量。
   手动核对 `settings.json` 中的程序路径、参数、环境配置和外部目录引用，
   特别是指向已移动托管安装目录的绝对路径。不要批量替换 Agent 原生配置或历史记录。
   若选择保留自定义数据位置，应使用新环境变量或现有命令行选项显式指定它。
   更新桌面规则中的 app_id；第三方 gRPC 客户端需按新协议重新生成并部署，
   自定义 ACP 扩展也需更新上表中的 metadata key。
5. **安装并验证新版本。** 在新版本源码中运行 `make install` 安装 daemon、CLI、GUI，
   确认新配置和数据路径后再运行 `make install-bas-service` 启用新用户服务
   （此目标会立即启动服务）。检查 `systemctl --user status bas.service`、
   `bast agents`、`bast session list --all`，再打开 BasG 核对设置、会话历史与置顶状态。
   检查服务日志和后端路径，不要把“列表为空”直接当成旧数据已丢失。
6. **最后手动清理旧安装。** 验证完成后，根据 `nix profile list` 或原安装方式，
   只删除已确认的旧 profile 条目、旧可执行文件、旧桌面入口与旧服务单元；
   修改用户服务单元后执行 `systemctl --user daemon-reload`。
   保留备份直到确认无需回退，不删除外部 Agent 凭据或独立安装的工具。

安装器不会替你完成以上数据移动或旧安装清理。不要让新旧服务同时使用同一数据库；
需要回退时先停止新版本，再从独立备份恢复旧安装与数据，不混用两版客户端。

## 托管 Agent 安装的额外检查

托管注册不仅记录程序路径，还绑定清单的绝对路径与 SHA-256。
新版本只接受 `bas-installation.json` 这个清单文件名，见
[安装引用校验](../src/config/registration.rs)。仅移动外层数据目录并不足以完成迁移。

若保留安装目录的原绝对位置，可以在备份后只改清单文件名，并更新设置中的
`installation.path`；清单内容字节不变时，其 SHA-256 不变。
若重定位安装目录，清单内的启动命令和参数可能仍指向原位置。
这种情况下推荐在新版本中重新安装并注册对应 Agent：先备份设置，移除失效的托管注册，
保留有效的自定义配置，再由新安装生成清单及引用。不要只修改清单内容或绕过摘要校验，
也不要删除 Agent 独立管理的凭据。
