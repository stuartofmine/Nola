> **单版本开发约束：仅保留当前架构、接口和数据格式，不实现兼容层或数据迁移；不匹配的存储明确拒绝，不自动删除、重建或改写用户数据。**

# 持久 Agent、Room 与独立 Context

核对：2026-11-08。当前代码与产品方向必须分开：统一用户对话是目标；源码仍有 AgentRoom、成员 Context、Room submission/dispatch 和已有导航适配，不能文档化为已经删除，也不能继续断言主控路由完全未实现。

## 当前对象

- 可保存 Agent 是配置定义及 revision；Participant 捕获加入时的身份/默认配置，持有独立 Context conversation。
- `AgentRoom` 管理成员和 Room revision；会话通过现有 association 与它绑定。共享公开时间线不等于合并成员的私有模型历史、权限或 Run。
- Room submission 持久接纳用户意图，保留 submission ID、room revision、正文、可选目标成员以及当前合同中的 conversation/parent message 关联。
- 未明确指定目标时，已有 `routing_messages` 构造有界候选和结构化 participant_id 选择；Application 校验成员资格与原提交状态。模型不创建 Context 或自行授予执行权。
- 明确目标或完整专家集合仍必须核对各成员、版本和准入；单成员成功不代表所有成员已接纳或执行。聚合提交状态与各成员 Run 终态分开。

## 归属与恢复

成员模型历史、草稿、上下文、授权、Run 和取消分别跟随精确 Context/执行 owner；父协作或临时 subagent 不能共享一个全局可变 session。持久 dispatch、接纳回执和各 Run 绑定负责恢复，内存 participant selection 不能代替正式提交记录。

修改保存 Agent 不静默重写已加入对话的配置快照。移出成员不撤销其已经存在的历史与待核对副作用；禁止借新增成员或切换 UI 复活旧执行权。

## 实现与目标

当前路径：[Room 用例](../../../../core/nola-application/src/agent_room.rs)、[submission/routing](../../../../core/nola-application/src/agent_room/submission.rs)、[分派契约](../10-Goal任务与子代理/ROOM_DISPATCH.md)。产品目标见[统一对话与可保存 Agent](../../../UI和交互设计/MULTI_AGENT_NAVIGATION.md)。

“删除独立 Room 产品入口”不能被当成已删除所有底层 Room 类型/发送协议的证明，也不要求维护新的旧入口兼容层。相应调用者调整必须与实际实现一起完成，不改写用户数据。当前静态接线、隔离测试、真实模型分派和各端操作需要分别提供证据。
