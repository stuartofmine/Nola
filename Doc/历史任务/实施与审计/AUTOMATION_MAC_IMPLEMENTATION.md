> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# macOS Automation occurrence 消费接线

状态：代码与回归源码已更新；未编译、未运行测试。最终统一验收前不可宣称 AUTO-02 完成。

## 事实来源

- 定义执行事实由 typed `execution_spec` 保存：prompt、workspace、recommended 权限模式、准入时默认模型策略、具体 context / skill 引用。原 metadata 只保存排程和 UI 信息，删除重复执行字段；读取由 spec 恢复显示模型。
- Mac 通过 `ObserveAutomationExecution` 取得四个稳定 identity，不自行推导其他 submission/Run。claim 携 occurrence / definition revision / worker / lease。
- occurrence 必须持久携带 `definition_revision`，Mac 从已提交定义读取版本并入队，Host lease 从入队回执读取版本。缺字段不可解码；修改定义不能使旧 occurrence 借用新版权限。事务入队、领取及准入由共享层验证版本，客户端预检查不代替原子校验。
- Host 请求携 `automation_execution_lease`、`automation_execution_spec` 和 `automation_artifacts`。个人/网页每个引用分别快照，Skill 使用活动 catalog 的实际 SKILL.md；列表记录 kind/id/path/SHA256。CLI 必须真实读取、核验再纳入上下文，声明不是来源证明。
- 不再经通用 `--context-file` 聚合上下文；不再用 `--yes` 覆盖 recommended；删除不持久化执行选项。
- 个人数据在可信 Mac 服务刷新成功后再生成快照。网页 snapshot 带 observed_at 与 maximum_age_minutes，不能把旧快照称为已刷新数据。旧 runner 子进程二次刷新已删除。

## 结果与通知

- 进程退出仅 `ObserveAutomationWorker` 事实。exit 0、日志标记和自行生成的 success watermark 均不能完成 occurrence。
- `ReconcileAutomationExecution` 返回该 occurrence 的完整 V2 seal；活动与通知完整保留六种业务结果。禁止读取最近 Run 替代精确绑定。
- 启动恢复使用 pending occurrence 游标分页，不受全历史固定条数限制。已绑定但未封存时通过公共 exactRun 恢复：核 occurrence binding、invocation digest、assistant identity 和 Application recovery inspection，再调用聊天共用的 `prepareColdResume` / `resume`。不重新 claim 或 Begin。
- 通知先原子写入本地 inbox，再 ACK 精确 notification_id。写失败不 ACK；已有相同 ID 幂等。ACK 表示本地展示历史已提交，不是用户实际看见，也不保证 OS 横幅 exactly-once。
- 初次运行也使用 `MacAgentHostRuntime.run`；Scheduler 不再拥有另一套 runner 脚本、模型参数或 Process 执行器。`MacLocalAgentRunRequest` 的 admission-only 字段携 lease/spec/artifacts，共用请求编码器生成 canonical `mac_launch`，实际 profile、协议、凭据引用、技能目录与权限冻结到原 Run。definition 级过程日志仅诊断，不作为恢复结果证据。
- 恢复从原 `mac_launch` 重建物理 Host，重新核验工作区授权及安全凭据引用；不会根据现在的 automation 定义或默认模型替换配置。lease/spec/artifact 路径不进入 resume 输入；已读取正文由原 checkpoint 持有。
- 自动化只授予原工作区路径；Skill catalog 的配置目录独立用于发现和读取 SKILL.md，不自动授予整个目录的文件或命令权限。Skill 附属资源通过普通 read_file/bash 使用时仍走具体资源授权。
- 持久问题等待、权限等待、未知效果以及已派发未确认的 effect 保持原状态，不自动授予、取消或重新执行。独立 occurrence 分别恢复，长运行不会阻塞通知分页。恢复结束仍须 Reconcile 得到原 Run seal 才能通知。

## 必须保留的验收与缺口

- Mac 真 SQLite 测试覆盖未绑定 occurrence、worker exit 0、reconcile 空结果、重开仍无伪造 seal。
- 通知失败不 ACK 的回归源码已补；共享层负责竞争 claim、准入绑定、封存与通知 ACK 持久化测试。
- exactRun 自动重开已接公共 Host；等待状态不自动回答，继续使用会话已有授权/问题处理入口。缺原规范 launch descriptor 的记录明确拒绝恢复，不从旧脚本或当前配置猜测。自动重开、等待后回答及重开竞争仍须统一实际验收。
- 内置 Skill 真模型/端到端 fixture 必须输出真实 ActionCompleted 要求与证据；不允许为保留旧测试而把 Answered 或 exit 0 放宽成行动完成。
- 仍需统一 Mac Test 配置执行、通知失败/重启故障注入、artifact 篡改/撤权/过期数据验收。此文档不替代验收结果。
