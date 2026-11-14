# Web

实现位于 [web/frontend](../../../apps/web/frontend)，Svelte 界面由用户拥有的 Rust Host 提供；浏览器不拥有第二套业务 Runtime，也不在浏览器本地执行桌面工具。

- [Web 构建目录](../../../apps/web/README.md)、[Host 传输](../../architecture/模块/14-远程Host与同步/README.md)。
- [会话交互](../../design/shared/CONVERSATION_UI_DESIGN.md)、[检查器](../../design/shared/CONVERSATION_INSPECTOR.md)。
- `npm --prefix apps/web/frontend ci`，然后运行 `run check`、`test`、`run build`。启动方式见[仓库 README](../../../README.md)。

公开/本地连接使用各自认证与 CSRF/信封边界，连接成功不授予工具操作。真实浏览器 → Host 200 步、压缩、取消及冷启动追问与[原型回放](../../design/prototype/docs/VALIDATION.md)分开记账，详见[实际范围](../../quality/VALIDATION_SCOPE.md)。

主 Agent 是唯一的对话入口。选择或 `@` 专家后，浏览器仍提交一次普通 MainRun；`selected_experts` 只携带稳定 ID/版本与已有成员、上下文引用，不拼接提示词、不另建用户消息。Host 将其转换为 Core 校验的 `main_expert_selection` 类型上下文；传输重试继续使用原冻结请求。

主 Agent 通过 `delegate_expert` 调度专家的独立模型、档位、Skill 和权限，执行沿用普通 subagent 的 Task worker。专家是叶子任务，不互调、不再生成 subagent，也不直接向用户追问；任务结果和缺失信息由主 Agent统一处理。界面只为主 Agent 提供发送入口，专家上下文只读并挂在主会话目录下。Windows Web 容器沿用同一行为；本机未执行 Windows 原生验收。

专家 chip 表示可取消的会话选择，普通准入消费该次正文与附件草稿后仍保留选择；取消 chip 不移除已加入成员或删除其上下文。Main 自行决定是否需要调用所选专家。

专家叶子不提供 `request_user_input`，缺失信息由主 Agent 统一向用户确认。旧冻结专家配置若含主控控制则明确拒绝恢复执行，保留历史和可用取消操作，不改写旧捕获。
