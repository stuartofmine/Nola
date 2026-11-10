> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；不保留旧版本兼容、双读双写或历史数据转换。不匹配的协议或存储格式明确拒绝，不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Harness 上下文投影：动态发现、冻结证据与真实请求

状态：2026-10-12 已进入最后统一验收；共享 Rust 检查及 Core 库测试通过，CLI 首轮
暴露输出预览问题，批量修复中。真实模型与客户端全链路尚未完成，详见 VERIFICATION。

## 对照 Cursor：借鉴机制，不照抄闭源实现

Cursor 的[动态上下文设计](https://cursor.com/blog/dynamic-context-discovery)将大工具结果、历史、Skill 和 MCP 定义作为可按需读取的上下文，而不是全部常驻 prompt。Nola 已有会话隔离的工具原文存储、历史召回、deferred catalog 和 typed activation，应完善现有所有权，不再平行造一份文件目录充当权限真相。

Cursor 的[harness 演进说明](https://cursor.com/blog/continually-improving-agent-harness)强调减少硬编码干预、按工具和模型分类观测错误，并把 token/cache 指标与完成质量分开评估。这里参考的是公开的一手说明，未取得或审查 Cursor 闭源源码；其产品指标不能直接作为 Nola 的收益或性能预算。

## 本次发现的架构缺口

1. 原 `AgentModelToolProjection::capture` 对 schemas 副本排序再计算 digest，但 `InvokeModel.tools` 仍使用原数组；Host 校验也容忍重排。由此相同 prefix identity 可能对应不同工具数组顺序。逻辑工具集合相等，不等于实际请求前缀相等。
2. planner 每轮向历史中最近的工具结果追加 `_nola_available_capabilities`。即使不修改 canonical transcript，也会改写下一次模型请求已经发送过的前缀；实时工具可见性与历史证据混成了第二份事实。

## HCTX-01：单一工具投影编译边界

| Owner | 输入 | 输出 / 不变量 |
|---|---|---|
| Host capability catalog | 实际执行能力、schema、可用性、权限元数据 | 只声明能力，不从用户措辞推断授权 |
| Core planner | 冻结的 Turn catalog、模型调用能力、typed activation、当前控制阶段 | 候选 schemas；已有权限边界不因发现工具而放宽 |
| Core `AgentModelToolProjection::compile` | Step revision、Turn projection digest、候选 schemas | 同时返回实际有序 `tools` 和其 `projection`；名称唯一且无空白归一化歧义 |
| Core effect dispatch | 编译结果 | 原序传递 tools，冻结 StepContext 与待执行请求；不能单独排序哈希副本 |
| Host model adapter | 已提交的 ModelStream + StepContext | 严格核验同一有序工具数组、digest、估算成本，然后执行协议编码 |

编译采用确定性的 capability ID 顺序与对象 key 顺序。重复、缺失、空白前后缀或冲突的名称拒绝整个编译，不去重猜测谁覆盖谁。当前返回 reducer 错误，不产生可执行的模型请求；Application 的事务回退保留此前 canonical 状态，不启动一次“修复 catalog”的模型调用。

`schema_token_estimate` 从同一实际数组计算，只是预算估算，不能冒充 provider 计费 token。序列化的 Step projection 与 pending request 随原有 Core checkpoint / Application 提交保存，不添加第二个缓存数据库。恢复后 Host 仍严格验证；不兼容旧的“数组任意重排”契约。

编译后的 Core schemas 不是 provider wire body。协议编码、provider route、system/messages/thinking 等仍可能改变物理请求；热分叉资格必须使用 `CONTEXT_AND_ACCOUNTING.md` 的已捕获物理快照，不能仅凭 tool digest 声称命中 KV cache。

## HCTX-02：实时能力与历史证据分离

实时可调用集合只来自当前编译的 schemas / Step capability IDs。typed Host activation 更新已冻结 Turn 的受控能力投影，并在后续 Step 暴露选中 schemas；不将完整 catalog 复制到每轮消息，也不把实时 catalog 填回旧 ToolResult。

已完成的 ToolResult 保留原有证据语义；它提及的工具不自动成为本步可调用工具。原有 `model_tool_not_exposed` 恢复和 typed discovery 继续工作，权限照常由具体调用及资源判定。搜索、当前、memory 等自然语言不成为路由或授权信号。

新增能力本身会改变工具前缀，这是明确的 cache 失效原因，不能承诺动态发现永远命中。能力集合不变、仅发现结果顺序不同，不应改变工具数组。不要为提高缓存率隐藏实际权限或保留已撤销能力。

## HCTX-03：有界工具证据与续读

Host 首次接收大输出时先把完整原文存入原会话的 ToolOutputStore，随后生成冻结的
typed preview；后续 Turn 不反复改写已发送结果。正文短不代表结构化 data 小，二者
都必须进入尺寸检查，不能只截 display text 而让巨大 data 再次进入模型请求。

输出 envelope 持有唯一 full_output_ref、truncated、原尺寸以及精确的 UTF-8
preview_end_byte_offset。续读必须使用真实原文字节偏移，而非字符数、预览 JSON
长度或猜测页长；对 JSON 转义、中文与裁剪后的 preview 也成立。导航说明不重复路径
或整份工具 schema，必须为实际证据保留预算。用户只需局部事实时按需搜索／读取；
任务明确要求全量覆盖时才穷尽必要范围，未读部分不得冒充完整证据。

执行回执、typed activation 和来源控制信息独立于正文预览，不得因为省 token 丢失
执行不确定性或扩大权限。原文保存失败返回明确的不完整证据与未知副作用提示，不能
把缩短后的成功文案当作安全重试依据。受控元数据自身超过预算时也不得静默伪造完整
输出；这类情况需要单独测量并报告，不能以空 preview 宣称优化成功。

外部工具与内置工具遵循同一原文保存失败语义：`full_output_available:false` 和
明确的 preservation error，而不是仅有 `truncated:true`。没有可读原文时不提供
续读偏移或伪造引用；保留来源验证要求，明确提示修改可能已经生效、重试前先检查。
这属于输出证据保存失败，不回写或伪造工具执行回执。已补不可用存储的隔离回归，
本批完整库验收尚未返回。

## 验收与性能观测

已补源码断言，统一最终阶段再运行：

- 不同输入排列编译为同一实际 tools、digest、IDs 与估算；Host 拒绝编译后重排、增删、schema 篡改与估算篡改。
- 重复名称、无名、空名、空白名称、冲突名称均拒绝；空 tools 是合法的纯文本模型投影。
- Core typed activation 的真实 HostRequest 有稳定工具顺序、可通过自身 Step witness 校验，历史消息不再被追加 live catalog 字段。
- 序列化恢复仍能校验同一数组，不以重建不同排序的数组掩盖不一致。

最终真实会话另需记录：每 physical attempt 的实际输入量与 cache-read coverage、schema 估算、发现次数、工具错误类别、请求与上下文提交耗时，以及最终是否完成用户任务。缺测仍是 unknown，不把估算当计费、不用减少工具数量代替任务完成质量。当前尚未取得本次变更的真实缓存或延迟收益。

后续主干审计：模型切换后的协议快照失效、长结果原文召回闭环、每种错误的可恢复性与未知副作用边界、压缩前后工具/消息 wire 前缀对照。这些必须连接现有 Run/Goal/usage ledger，不再创建另一套 harness 状态机。

## 摘要输入中的公开语义与 provider continuation

共享 `CompactionSummaryPlan` 在打包摘要请求前，只从顶层 `type=reasoning` 的模型上下文投影中移除 `provider_continuation`。保留原 id、summary 和 content；空公开文本的 reasoning 仍保留身份，不会将全 opaque 的档案误判为空。消息正文与工具数据中同名的任意字段不做递归删除。

加密/签名的 provider continuation 是原生请求接续材料，不是文本摘要器可理解的正文。该投影不改 canonical transcript、来源指纹或原始恢复记录，也不改变正常请求对 Preserve/ClearOnCompaction 的处理；Preserve 同样不表示把 opaque 信封作为 JSON 文本交给摘要器。新 summary plan 和其冷恢复 checkpoint 都只包含语义投影，不额外保存被排除的信封。

以 2026-11-09 三题复测中的 181 条来源离线重放新旧实际打包器：移除 35 个 continuation 信封，摘要 archive segment 从 394,025 降为 227,493 UTF-8 字节，减少 166,532 字节（42.26%）。其余投影字段逐条一致，原始档案字节不变。这是该档案的输入字节减少，不能当作 token、费用或最终上下文压缩率的同等改善。

已采纳审查意见的原反例、修复版本、最终资源检查与未解决状态也必须在摘要中保留来源。其运行时权威仍是结构化 requirements/观察状态；摘要或子任务的成功文字不能覆盖 pending finding。
