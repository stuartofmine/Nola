> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 生成参数覆盖与最终执行合同

状态：现行生成参数合同；2026-11-08 更新原生推理档位与作用域。seed 阶段证据保持其 2026-09-28 覆盖范围，不扩大为全部客户端/真实 provider 已验收。
隶属 [整体模块化改造](../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md)，不替代完整 HostInvocation 目标。

## 所有权

会话保存的是用户意图，模型中心保存的是模型默认值与声明能力，Application
解析一次，Host 只消费冻结的最终值。Presenter 不根据默认数值相等推断参数
是否应该发送；Provider 不重新读取当前全局设置，也不把 null 当作任意默认值。

配置补丁、持久化偏好、最终生成配置是三个不同合同，不能共用一个 Optional。
尤其 `seed=0` 是合法值，不是禁用标记。

## Seed 三态

持久化的每模型会话覆盖使用以下语义：

| 意图 | 持久化 seed 字段 | 配置 patch | 最终 seed |
| --- | --- | --- | --- |
| 继承 | 缺字段 | `null` 删除字段 | 取模型默认值 |
| 清除 | `{"kind":"clear"}` | 整体替换该字段 | 无值，不发送 |
| 指定 | `{"kind":"value","value":"0"}` | 整体替换该字段 | 有符号 64 位整数，包括 0 |

value 使用规范十进制字符串，避免 Web JSON Number 在 2^53 之后丢失精度。
仅接受 `0` 或无前导零的正/负整数；不接受 `+1`、`-0`、空白、小数、指数和
越界值。clear 禁止携带 value，value 禁止未知字段；缺少 tag 不猜测含义。

Domain 拥有类型、规范编码与边界校验；Application 拥有继承解析。
Store 的通用 null 删除语义保持不变，不在 Store 中引入生成参数策略。
完整 settings 写入与 sparse patch 必须调用同一 validator，避免一个严格、
另一个任意 JSON 的旁路。

最终 HostGenerationBinding 保持 resolved optional integer，不携带继承标记。
远端请求若继续采用 sparse generation_options，clear 必须显式编码为
`generation_options.seed:null`，inherit 则不提供该键，value 为精确整数。
编码器不得经过 Double/JavaScript Number 中转大整数。

## 条件采样与思考档位

先冻结模型能力、会话参数与显式推理意图，再解析 reasoning selection，最后决定
是否发送采样字段。声明只在 effort=none 支持采样的模型，解析到 none 时应
发送冻结值，即使恰好等于 UI 的 0.7/0.9；解析到其他 effort 时，默认采样
不发送，显式不兼容覆盖则拒绝，不能静默丢失用户意图。

这不是所有模型的统一能力。官方 [GPT-5.2 参数兼容说明](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-5.2)
给出该条件；Nola 应由模型能力合同表达，不以模型名/URL 字符串硬编码路由。
本地参考 `参考资料/codex-main/codex-rs/codex-api/README.md` 的 adapter 边界
用于区分 wire 序列化与应用配置决策，并非直接复制其产品默认值。

## 模型原生档位与公开推理摘要

当前聊天选择器直接展示模型声明的实际 effort。Mac 本机、iOS、Android 与 Web 已接入精确档位；五档不会压缩为简单/中等/复杂，也不与原三档菜单并列。`reasoningTier` 仍用于底层显式 agent 策略意图，不作为新的聊天推理入口。

| 配置/请求 | 含义 |
| --- | --- |
| 未设置 `reasoningPreference` | 继承模型默认；不得在读写往返中自动改成 `automatic` |
| 会话 sparse patch 的 `reasoningPreference:null` | 删除覆盖，恢复继承；不是 provider 的物理参数 |
| `reasoningPreference:"automatic"` | 显式使用提供商默认；新的 UI 保存同时清除冲突 tier |
| 实际 effort 字符串 | 按该模型当前声明校验，生成唯一精确 selection；未知或不支持值拒绝 |
| `reasoning_selection` 与非空 `reasoning_tier` 同时进入直接解析请求 | 冲突并拒绝，不让 Host 自行选择优先级 |

会话翻译器 `session_override_request` 对精确 effort 移除 tier，再交共享解析器；profile 默认偏好仅在请求没有覆盖时生效。已有 `automatic` 与显式 agent tier 的组合仍按 resolver 的现行合同处理，不能把标签改成“默认”就宣称请求已去掉 tier。配置编辑必须提交真实 patch。

Mac 本机请求不再补写隐式 `balanced`。连接 Host 的模型仍由源 Host 声明及准入，不能用本机模型目录投影能力；原生精确菜单的实际范围与 budget UI 差异见[客户端配置接线](../13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。

协议适配只序列化最终 selection：Responses 使用 `reasoning.effort`，Chat Completions 使用该协议的 effort 字段，Anthropic budget 按已准入 token 值处理。保存一次选择不等于探测能力，也不保证 provider 实际接受；目录、能力与凭据失败需如实报告。

effort 支持和公开摘要参数支持是独立能力。目录明确提供的
`supports_reasoning_summary_parameter` 经 profile 的 `reasoningSummarySupported`
保存为可空布尔值并随 Host 调用冻结；`false` 时两个 Responses 路径均不得发送
摘要参数，即使 effort=high。启用推理且支持摘要时 API-key 与 SIWC 都显式请求
`reasoning.summary=auto`；未知 SIWC 能力不自动请求，普通 API 的未知值保留既有
显式 thinking 设置。恢复、子代理及压缩 continuation 不得丢失已知 false。
公开摘要由服务实际返回，
不是原始思维链，也不保证每次请求都有摘要。UI 不得通过计时伪造思考。
依据：[Reasoning summaries](https://developers.openai.com/api/docs/guides/reasoning)、
[SIWC inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)。

## 贯通实现与验收

1. Domain seed override 类型及 Application 的两类写入校验；Store patch 仍
   整字段替换或删除。旧数字表示不能被静默解释成新三态。
2. Mac Codable、三态设置 UI、applying、保存重开与 Remote intent 编码。
   本地请求只拿最终 ModelParameters，不重新继承模型默认值。
3. CLI 会话覆盖翻译与 Web 编辑器同步使用精确十进制 codec；检查其他平台
   settings 消费点，不能向仍接受数字的端发送 tagged object。
4. profile seed=42 × inherit/clear/0/负数/i64 边界矩阵，覆盖 CAS、重开和
   patch reset；越界及非规范输入拒绝，失败不得修改已提交配置。
5. 验证本地/远端实际请求：继承发送 42、清除省略、指定 0 发送 0。
   Responses 不支持 seed 的情况下，clear 后合法，继承非空则准入拒绝。
6. 条件采样必须同时验证 Application→Host payload 和最终 HTTP body。
   helper 返回布尔值的单测不能独自证明跨进程执行保持一致。

## seed 阶段证据与剩余边界（2026-09-28）

Domain `generation_override::SeedOverride` 拥有严格 tagged codec；Application
的 patch 与完整 settings 写入均校验模型覆盖。完整快照拒绝 seed:null，
它只作为 patch 删除操作存在。Store 验证整字段替换、CAS 与重开；不解释 seed。
Mac 用 `MacSeedOverride?` 区分三态，Web 使用 tagged 字符串且仅以 BigInt
检查范围；CLI 将偏好解析为最终 numeric i64/null，而不是把 tagged 对象发给模型。
直接 iOS ModelParameters 仍是 resolved 参数快照，不额外引入继承状态。

Mac repository 不再将非法执行配置解码错误吞掉后降级为默认值。缺少原生
version/isPinned 的跨端稀疏设置只补展示默认值，模型覆盖仍严格解码。
单会话读取明确抛错；批量档案读取仍记录并跳过不可读会话。未提供旧 numeric
偏好的自动迁移，也不会自动删除用户数据；旧非法值不能继续静默运行。

本轮已有实际验收：Domain 2/2，Application 两组共6/6，Store 3/3；CLI
seed过滤5/5、host_model_intent过滤7/7（存在重复覆盖，不能相加宣称12个独立测试），
包括最终本地HTTP抓包的精确整数与省略字段验证。Web 定向8/8含Svelte组件编译，
全量svelte-check为0 errors/0 warnings。Mac最终集成72/72通过，Application
会话单元回归25/25通过，具体报告路径记于整体文档。

这些是隔离数据库与受控HTTP fixture 验收，不等同联网真实模型或用户点击的
跨设备端到端验收。模型中心 resolved seed 的 Web大整数输入、远端记忆策略、
完整HostInvocation持久化等仍属未完成的整体改造目标。
