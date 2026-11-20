# Web

实现位于 [web/frontend](../../../apps/web/frontend)，Svelte 界面由用户拥有的 Rust Host 提供；浏览器不拥有第二套业务 Runtime，也不在浏览器本地执行桌面工具。

- [Web 构建目录](../../../apps/web/README.md)、[Host 传输](../../architecture/模块/14-远程Host与同步/README.md)。
- [会话交互](../../design/shared/CONVERSATION_UI_DESIGN.md)、[检查器](../../design/shared/CONVERSATION_INSPECTOR.md)。
- `npm --prefix apps/web/frontend ci`，然后运行 `run check`、`test`、`run build`。启动方式见[仓库 README](../../../README.md)。

公开/本地连接使用各自认证与 CSRF/信封边界，连接成功不授予工具操作。真实浏览器 → Host 200 步、压缩、取消及冷启动追问与[原型回放](../../design/prototype/docs/VALIDATION.md)分开记账，详见[实际范围](../../quality/VALIDATION_SCOPE.md)。

如果旧版会话的手动压缩一直待完成，可在对话底部选择「刷新」查看最新状态，或选择「复制整段会话」并确认。左侧会话的「⋯」菜单也提供复制入口。新会话包含已完成的消息，可以继续提问。原会话保留原有记录和压缩任务。如果旧版创建的副本已继续对话，但模型读不到复制过来的历史，请在升级后打开这个副本，再复制一次，即可将可见的原始消息和后续消息一起带入新会话。

主 Agent 是唯一的对话入口。选择或 `@` 专家后，浏览器仍提交一次普通 MainRun；`selected_experts` 只携带稳定 ID/版本与已有成员、上下文引用，不拼接提示词、不另建用户消息。Host 将其转换为 Core 校验的 `main_expert_selection` 类型上下文；传输重试继续使用原冻结请求。

主 Agent 通过 `delegate_expert` 调度专家的独立模型、档位、Skill 和权限，执行沿用普通 subagent 的 Task worker。专家是叶子任务，不互调、不再生成 subagent，也不直接向用户追问；任务结果和缺失信息由主 Agent统一处理。界面只为主 Agent 提供发送入口，专家上下文只读并挂在主会话目录下。Windows Web 容器沿用同一行为；本机未执行 Windows 原生验收。

输入时在光标处选择专家，已发消息在原句保留名字。下一条草稿可重新点名；当前会话中已加入的专家和子会话继续保留。交互与恢复规则见[消息内的专家提及](../../design/shared/EXPERT_MENTIONS.md)。

专家叶子不提供 `request_user_input`，缺失信息由主 Agent 统一向用户确认。旧冻结专家配置若含主控控制则明确拒绝恢复执行，保留历史和可用取消操作，不改写旧捕获。
