# Bas 设计文档

Bas是一个各种agent服务的守护进程

## 架构
我们守护的会话分为以下几种
1. 嵌入式agent：一个我们使用rust自己开发的agent工具，可以嵌入到Bas的代码中。
2. 外部驱动
  a. ACP （Agent Client Protocol）外部服务，可以通过作为一个ACP客户端来驱动外部的Agent侧实现，如opencode/codex之类

## 会话
一个会话具有如下能力

1. 具有属性，可以检索：会话应该有uuid作为唯一id。具有一些业务强相关的属性：如agent端id，对应的配置，发起的用户与主机等。具有一些业务弱相关的属性，如标题等。
2. 可以被前端detach, 可以attach。前端detach的时候不影响agent loop继续推进。在重新attach的时候，我们要按照前端要求播报最近的N条消息，并且能够允许前端依照游标来加载历史消息。
3. 在d层消除底层格式差异。

## 持久化
Bas负责保存会话元数据、ACP事件与生命周期事件，并提供按游标回放。Agent负责保存自身可继续执行的上下文；
这两种持久化相互独立，只有记录而没有可恢复的Agent上下文时，不能把恢复出的会话宣称为可继续对话。

`--storage memory|localdb`（环境变量`BAS_STORAGE`）选择存储后端；默认`localdb`持久化
新会话，`memory`用于明确不保留会话的部署。`localdb`使用Toasty的异步SQLite驱动及版本化迁移，
`--localdb-path`（环境变量`BAS_LOCALDB_PATH`）可覆盖数据库位置。
两种后端遵循相同的事件顺序与提交契约：成功响应、提交确认和广播只能发生在存储提交成功之后。
内存历史可以按容量淘汰；localdb历史由数据库持久保留，不因实时回放窗口淘汰而丢失。

恢复已有会话时，Bas使用持久化的私有后端标识调用对应Agent的恢复操作，保持公开会话ID和事件游标不变。
不支持恢复的Agent应返回明确错误，不应偷偷创建一个没有旧上下文的新会话。重启前正在执行的turn不能自动重放，
否则可能重复执行外部工具。

ACP恢复优先使用`session/load`；只支持`sessionCapabilities.resume`的Agent使用`session/resume`。按
[ACP会话加载规范](https://agentclientprotocol.com/protocol/session-setup#loading-sessions)，`session/load`
会在响应前回放旧消息，Bas应丢弃这些重复更新，仅在加载完成后接收新的更新；`session/resume`不回放旧消息。
当前嵌入式后端是无状态Echo占位，不具有可持久化的多轮模型上下文。用户可见的事件历史不是未来真实嵌入式Agent
上下文检查点的替代品。

## 对接linux生态
### systemd
systemd是我们首先需要对其的一个环境，通过systemd我们可以在很受限的环境中持续为用户提供agent的能力。

在systemd的条件下，我们可以通过journald来写日志。

### D-Bus
我们在完成一些任务后可能需要通知用户任务完成，这样可能需要调用一些桌面环境的通知功能。

## 作为服务
我们要对外提供服务能力，供ctl和客户端（如tui/gui）调用，需要考虑使用gRPC作为向外提供调用能力的渠道。
