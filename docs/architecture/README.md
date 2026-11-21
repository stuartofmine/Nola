# Nola 软件架构设计

简体中文 · [English](README.en.md)

用户发送消息时，客户端向 Application 提交命令，Agent 引擎提出状态转换，Store 提交后由 Host 调用模型或执行工具，客户端再读取已提交的 journal。在手机观察配对宿主的任务时，Run 仍归源宿主。排查故障时，先根据这条链路确定哪个组件拥有待核对的事实。

## 架构指南

| 入口 | 内容 |
| --- | --- |
| [系统设计](总体设计/SOFTWARE_DESIGN.md) | 一次输入如何变成持久消息、模型请求和工具效果 |
| [产品与架构要求](总体设计/REQUIREMENTS.md) | 跨端场景、REQ-ARCH 与 REQ-RUN 验收条件 |
| [部署与源码映射](总体设计/CURRENT_IMPLEMENTATION_MAP.md) | 六个客户端、执行进程、原生制品与各模块入口 |
| [Rust 声明依赖](总体设计/DEPENDENCIES.md) | Cargo 工作区直接生产依赖与实际耦合 |
| [命令与模块契约](模块契约.md) | 请求身份、事务回执、版本冲突、错误归属 |
| [执行与故障恢复](总体设计/跨模块流程与恢复.md) | 编辑重发、长任务、压缩、断线与取消 |
| [模型文件与启动](模块/08-模型配置与调用/MODEL_FILE_CONFIGURATION.md) | 文件优先级、事务发布、凭据绑定与失败行为 |
| [API 入口](模块/01-领域与协议/API.zh-CN.md) | Rust Host HTTP、Service 命令和嵌入式端口 |
| [版本与数据兼容](总体设计/VERSION_COMPATIBILITY.md) | 1.0.0 基线、版本协商、可回滚迁移与旧数据保护 |
| [分发与更新](总体设计/SOFTWARE_UPDATE_ARCHITECTURE.md) | 签名 feed、目标制品、平台安装交接 |

- [所有权](总体设计/架构决策与所有权.md) · [Core、Application 与 Host](总体设计/CORE_HOST_ARCHITECTURE.md) · [部署分层](总体设计/CORE_HOST_LAYERING.md)

## 模块

每个模块说明状态归属、输入输出、失败处理与源码入口。编号划分职责，一个模块可以涉及多个 crate。

- [MOD-01 · 领域与协议](模块/01-领域与协议/README.md)
- [MOD-02 · Core与状态机](模块/02-Core与状态机/README.md)
- [MOD-03 · Application与事务](模块/03-Application与事务/README.md)
- [MOD-04 · 存储与索引](模块/04-存储与索引/README.md)
- [MOD-05 · 执行与恢复](模块/05-执行与恢复/README.md)
- [MOD-06 · 工具与证据](模块/06-工具与证据/README.md)
- [MOD-07 · 权限与平台](模块/07-权限与平台/README.md)
- [MOD-08 · 模型配置与调用](模块/08-模型配置与调用/README.md)
- [MOD-09 · 上下文与压缩](模块/09-上下文与压缩/README.md)
- [MOD-10 · Goal任务与子代理](模块/10-Goal任务与子代理/README.md)
- [MOD-11 · Skill插件与MCP](模块/11-Skill插件与MCP/README.md)
- [MOD-12 · 记忆数据源与自动化](模块/12-记忆数据源与自动化/README.md)
- [MOD-13 · 客户端投影](模块/13-客户端投影/README.md)
- [MOD-14 · 远程Host与同步](模块/14-远程Host与同步/README.md)
- [MOD-15 · 观测与验收](模块/15-观测与验收/README.md)

## 专题与验收

主线架构指南和 15 个模块均可切换中英文。补充规范和带日期的审计保留原文。源码入口用于确认实现位置；验收范围记录实际运行的设备、模型与场景。例如，提交回执能验证事务成功，回到页面能否继续输入需要在目标客户端操作验证。

- [所有权决策](总体设计/架构决策与所有权.md) · [Core 与 Host 分层](总体设计/CORE_HOST_LAYERING.md) · [执行权限与证据](总体设计/执行权限与证据边界_20261016.md)
- [运行时接口](模块/01-领域与协议/CURRENT_RUNTIME_INTERFACES.md) · [A2UI 运行时](模块/13-客户端投影/A2UI_RUNTIME_CONTRACT.md) · [会话聚合](模块/04-存储与索引/CONVERSATION_AGGREGATE_CONTRACT_20261015.md)
- [交互与原型](../design/README.md) · [各平台接入](../platforms/README.md) · [验收范围](../quality/VALIDATION_SCOPE.md) · [11 月 8 日源码审计](总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)
- [Rust 构建缓存](工程规范/RUST_BUILD_CACHE.md) · [跨语言编码规范](工程规范/CODING_STANDARDS_RUST_SWIFT_KOTLIN.md) · [测试架构](模块/15-观测与验收/TESTING_ARCHITECTURE.md)
