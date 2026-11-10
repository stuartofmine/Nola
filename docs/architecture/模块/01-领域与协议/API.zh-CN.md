# Nola 当前接口文档

核对：2026-10-28。当前接口分为三种边界，不能共用一套未经区分的请求/认证模板。

| 边界 | 入口与职责 | 契约与字段 |
| --- | --- | --- |
| Rust Host HTTP | `cli/src/web_parts/routes.rs`；本机Web、配对移动端、模型管理与运行观察 | [当前运行时接口](CURRENT_RUNTIME_INTERFACES.md)；[固定路由及源码](INTERFACE_CATALOG.md#rust-host-固定路由) |
| Core Application | `ApplicationCommand`；客户端/Host经Service或embedded端口提交 | [命令tag及类型入口](INTERFACE_CATALOG.md#application-顶层命令-tag)；[Application端口](../03-Application与事务/APPLICATION_PORT_CONTRACT.md) |
| Cloudflare Worker | `backend/worker/src/index.ts`；身份、渠道、模型代理、源/结果、通知及replica传输 | [当前Worker分发](INTERFACE_CATALOG.md#worker-路由)；不等于本地执行Host或Core reducer |

## 使用规则

- 本机Web默认入口`http://127.0.0.1:5001/api`；具体监听与本地信任策略由Host配置决定。Worker部署地址不等于本机地址。
- 配对移动接口沿用Bearer认证与加密信封；源Host、client、conversation及原Run/assistant身份分别核对。Web会话、Worker账户、渠道webhook和admin鉴权各按其handler执行，不互换凭据。
- JSON、SSE与分块journal是不同传输。SSE断开不提交取消；先查询原提交回执并按精确cursor补齐。HTTP 200/202不自动代表命令已修改配置、模型已完成或工具已停止。
- 修改请求使用领域要求的CAS/固定操作身份；重复同键同内容返回原事实，同键异内容拒绝。未知结果保持未知，不换ID重发副作用。
- A2UI v0.9.1、Application Service协议、C ABI、HTTP v2及各DTO protocol_version独立管理。字段大小写按具体类型，不能全局改名；没有旧格式回退。
- 当前接口目录列出实际route/dispatch和命令映射，并链接字段/返回类型源码。动态路径必须结合parser/handler，不能把prefix匹配概括成任意资源授权。

## 重点输入输出

提交frontier、会话选模、child准入/唤醒/恢复、Goal控制能力、审批重示、版本边界及远程usage/journal编码见[当前运行时接口](CURRENT_RUNTIME_INTERFACES.md)。模型、Skill、Plugin、Memory及移动FFI入口见[领域用例与桥接](INTERFACE_CATALOG.md#领域用例与平台桥接)。

旧Worker/OAuth列表移至历史接口基线（本地工作记录，不随仓库分发），不再作为现行接口列表。本文为源码核对，不是全路由端到端测试报告。

[English](API.md)
