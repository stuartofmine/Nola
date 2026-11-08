> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-14：远端 Host / 同步 / 后端

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- 同一业务记录只有一个绑定的 authority；本地 Store 对本地业务权威写入，对远端执行事实保存认证副本。
  远端复制不授予本地执行权，源事件与派生投影分开，不用“本机持久化”声称拥有远端 Run。
- 输入：认证主体、版本化 envelope、源 owner/revision、幂等键；输出：提交 ACK/冲突/范围受限投影。
- 断线后按 cursor 补齐，不复用新 Run 接纳旧回执；权限决策跨设备必须明确 authority。
- 后端中转/同步不能成为另一套 agent 状态机或凭据明文仓库。
- 验证：断线重连、重复/乱序、源设备删除、撤权、跨会话事件与不同 schema 拒绝。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

远端传输适配器只处理认证、信封与复制；同步投影不是本地执行所有者。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [routes.rs](../../../../cli/src/web_parts/routes.rs) | Rust Host 的 Web/paired mobile 入口 |
| [index.ts](../../../../backend/worker/src/index.ts) | Worker 身份、代理、渠道与同步分派 |
| [README.md](../../../../transport/tailcat/README.md) | 可选隧道与私有管道、身份及生命周期合同 |

Rust 执行 Host、Worker/D1、远端镜像与可选 Tailcat 是不同角色。可选隧道替换承载，不取代配对/登录/CSRF/HMAC、源 Run 权威或恢复游标。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [REMOTE_HOST_PROJECTION_CONTRACT](REMOTE_HOST_PROJECTION_CONTRACT.md)
- [WEB_CHAT_TRANSPORT_CONTRACT](WEB_CHAT_TRANSPORT_CONTRACT.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

## 当前源提交与复制接口（2026-10-28）

源Host发布submission frontier及会话execution selection，客户端冻结配置/父消息前置条件并核对原owner。context usage与journal以原摘要复制，嵌套协议字段不能被统一改名。并发迟到内容只有精确已提交记录与manifest匹配才幂等确认。

具体字段、返回值、错误和源码索引见[当前 Core 与 Host 接口](../01-领域与协议/CURRENT_RUNTIME_INTERFACES.md)。本次仅核对文档，不新增运行验收结论。

## 可选 Tailcat 传输：当前角色

可选承载已有实现，不再把整个集成描述为“移动平台绑定尚待开始”。固定源码和构建要求以 [transport/tailcat](../../../../transport/tailcat/README.md) 及其 go.mod/sum 为准；这里不维护“上游最新版本”的第二份数字。

- `--tailcat-ingress` 改变远程入口认证语义，不独自启动隧道或生成永久身份。loopback 转发不能继承本机 Web 信任；保留 Web 登录/CSRF 与移动 HMAC/加密信封。
- helper/SDK 只承载一个精确 Nola HTTP/SSE 服务，握手核对 expected Host identity；连接地址/PSK 不是普通 URL、argv 或日志内容。可信 helper 路径/摘要、私有管道、公钥允许列表与临时 lease 各有校验。
- Apple 共享 `NolaTailcatTransport`，Android 普通/控制客户端复用其可选会话；是否实际链接 SDK 是构建事实。已宣告可用的承载连接失败不能自动降级、重发业务请求或放松证书策略。
- CLI 持有远程 Web 桥时，浏览器仍使用普通 HTTP/SSE 与原登录/CSRF，不在浏览器新增 WASM Agent。App/CLI 退出或断开传输不制造 Task/Run 取消 ACK。
- HTTPS、证书 pin、URL 前缀、前后台 park/resume、lease 到期和关闭未确认由具体 adapter 合同决定，不能由“本地端口已监听”假报可用。

临时身份不等于永久设备凭据；共享适配器存在不等于所有原生实窗、真机挂起、跨机器或长期冷重连已验收。连接源 Host 的执行权和原 cursor/receipt 仍须按正常恢复合同核对。SDK 可选性是当前平台部署，不是旧协议回退。

2026-11-06 起的安装、沙箱拒绝/获准、合成与真实 Host 测试流水已保存在[历史传输记录](../../../历史任务/实施与审计/TAILCAT_IMPLEMENTATION_EVIDENCE_20261108.md)。一次历史 EPERM 或解锁错误不能在当前规范中变成全局操作限制；本轮没有重试网络或系统配置操作。
