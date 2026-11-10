> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 远端 Host 工具目录冻结

状态：实施中，不能作为全链路已完成声明。

## 所有权与协议

Host 从实际配置、Skill/MCP 代际和可执行 schema 构造目录。客户端不按设置
开关猜测工具、不重算 Core 策略凭据。目录只证明 Host 的可用工具，不授予
具体资源权限；设备桥独立沿用现有广告及授权协议，不混入 AgentHost 目录。

认证 POST `agent/tool-plan/prepare` 输入 `expected_host_id/client_id/session_id`。
返回 `{source_host_id,result:HostToolPlanSnapshot}`；mobile 沿用加密传输。
snapshot 含 schema_version=1、reference、issued_at_ms、expires_at_ms、tools。
reference 含 manifest_id（Host UUID）、host_id、client_id、session_id、
catalog_revision（64 位 SHA 内容版本，不是单调时钟）。tools 每项包含
tool_id、route=agent_host、完整 OpenAI function schema。

prepare 使用有限容量、短 TTL 内存凭据库，进程重启后失效，客户端须显式
重新准备。提交语义载荷带 tool_plan_ref，Host 校验签发记录、身份、有效期
及当前目录版本，不接收客户端提供的工具内容。缺失/失配拒绝，不能降级。

同一已接纳 submission 的精确重投先调用既有只读 ReadSourceSubmissionReceipt：
重算原 semantic body 摘要并核验完整身份、摘要及原 Run admission cursor。
命中仅返回原 receipt，不进入 prepare、注册或执行；因此 TTL 到期或内存凭据
丢失不影响已接纳请求的观察。Prepared/未注册请求不能走此通道，仍要求新鲜
签发凭据；同 ID 不同 body 或不匹配摘要明确拒绝。

## 持久化与恢复

Run 通过现有 Application 创建事务将 typed snapshot 冻结进
AgentCoreConfig/run_invocation_payloads.config_json；与 Run 归属及原配置摘要
一起提交。没有额外 Run→ref 文件映射或第二个状态所有者。

认证 POST `agent/tool-plan/read` 输入上述三项身份及 expected_run_id、
assistant_message_id，调用 LoadExactRunInvocation 返回原 Run 快照。读取
原记录不要求目录仍在初始 TTL 内；TTL 限制新提交，不使长任务失去观察能力。
缺失快照或身份不符拒绝，不能从当前设置重建原 Run 目录。

该快照是 Run **初始目录**，不是永远不变的执行 allowlist。MCP 经 Core
批准连接后可以在该 Run 内激活新工具；新增能力仍受既有 Core 目录代际与
资源授权约束，不能由客户端声明。

权限事件携带 `tool_catalog_evidence={catalog_revision,tool}`，tool 与上面的
单项 schema 格式相同。Driver 从当前可用 descriptor 投影，必须与 Core
RequestPermission 的 catalog_revision 一致，随原 nonce/Host/client/session/
canonical_run_id 事件持久化。客户端核验原 Run 身份及当前事件 descriptor，
不以初始目录的 contains 检查拒绝合法新激活工具，也不重新读取设置猜测。
目录授权与操作授权分离；撤销后的执行由 Core 资源事实与 MCP generation
检查拒绝，历史显示凭据不能恢复权限。

MCP prepare 只使用 session-bound 已连接缓存及待连接描述，不主动启动服务。
时间为正的 JS 安全整数毫秒，TTL 最大 300000ms；digest 仅小写 SHA256；
身份及工具 ID 非空、无控制字符且 UTF-8 不超过 512 bytes。响应最大 512KiB。

## 验收

覆盖错 Host/client/session、缺失/过期/错 revision、签发记录遗失、工具及
MCP 代际变化、原 Run 冷读取、批准时撤销、权限 nonce 重放、设备桥分离。
目录建造和执行必须调用同一工厂，不能让 prepare 和 Turn 各自维护工具表。

## 当前观测与冻结配置（2026-10-28核对）

移动child准入的current_tools是当前Host观测，不替换冻结工具schema或权限。Android前台与embedded executor复用工具plan重验证；续租/重示只可使用同投递身份的已提交事实。计划过期不能被UI修改时间戳绕过，审批返回必须绑定原attempt/token/worker。
