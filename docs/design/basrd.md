# basrd 设计文档

basrd是一个各种agent服务的守护进程

## 架构
我们守护的会话分为以下几种
1. 嵌入式agent：一个我们使用rust自己开发的agent工具，可以嵌入到basrd的代码中。
2. 外部驱动
  a. ACP （Agent Client Protocol）外部服务，可以通过作为一个ACP客户端来驱动外部的Agent侧实现，如opencode/codex之类

## 会话
一个会话具有如下能力

1. 具有属性，可以检索：会话应该有uuid作为唯一id。具有一些业务强相关的属性：如agent端id，对应的配置，发起的用户与主机等。具有一些业务弱相关的属性，如标题等。
2. 可以detach, 可以attach
3. 尽可能消除底层差异。本地保存消息，回放。
